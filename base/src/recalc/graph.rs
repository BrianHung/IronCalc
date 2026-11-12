//! What every formula read the last time it ran, and who reads a position.
//!
//! Every position that holds a formula, or that a formula read as a single
//! cell, has a *node*. The nodes are in one vector and name each other by
//! their place in it, so going from a cell to the formulas that read it is
//! an index and not a search, and `Places` finds the node of a position.
//! Nodes are made as formulas first run, in the order of the sheet: formulas
//! that are neighbours there are neighbours here, and an evaluation that goes
//! down a column goes down the vector.

use std::collections::{HashMap, HashSet};

use crate::constants::{LAST_COLUMN, LAST_ROW};
use crate::evaluation::{CellKey, KeyBuild};
use crate::expressions::types::CellReferenceIndex;

/// The formulas an evaluation runs, in the order they were found. Which
/// formulas they are is marked on their nodes: `Graph::in_scope`.
#[derive(Debug)]
pub(crate) struct Scope {
    formulas: Vec<CellKey>,
    /// What the nodes of these formulas are marked with.
    mark: u32,
}

impl Scope {
    pub(crate) fn len(&self) -> usize {
        self.formulas.len()
    }

    pub(crate) fn formulas(&self) -> &[CellKey] {
        &self.formulas
    }
}

/// A rectangle of one sheet, both corners included.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) struct Rect {
    pub(crate) sheet: u32,
    pub(crate) row1: i32,
    pub(crate) column1: i32,
    pub(crate) row2: i32,
    pub(crate) column2: i32,
}

impl Rect {
    /// The rectangle with these corners, which are in the same sheet.
    pub(crate) fn between(left: CellReferenceIndex, right: CellReferenceIndex) -> Rect {
        Rect {
            sheet: left.sheet,
            row1: left.row,
            column1: left.column,
            row2: right.row,
            column2: right.column,
        }
    }

    /// The rectangle `width` by `height` whose top left corner is `corner`.
    pub(crate) fn sized((sheet, row, column): CellKey, (width, height): (i32, i32)) -> Rect {
        Rect {
            sheet,
            row1: row,
            column1: column,
            row2: row + height - 1,
            column2: column + width - 1,
        }
    }

    /// Grows the rectangle to hold a cell of its sheet.
    pub(crate) fn include(&mut self, (_, row, column): CellKey) {
        self.row1 = self.row1.min(row);
        self.row2 = self.row2.max(row);
        self.column1 = self.column1.min(column);
        self.column2 = self.column2.max(column);
    }

    #[inline]
    pub(crate) fn contains(&self, (sheet, row, column): CellKey) -> bool {
        sheet == self.sheet
            && (self.row1..=self.row2).contains(&row)
            && (self.column1..=self.column2).contains(&column)
    }

    fn intersects(&self, other: &Rect) -> bool {
        self.sheet == other.sheet
            && self.row1 <= other.row2
            && other.row1 <= self.row2
            && self.column1 <= other.column2
            && other.column1 <= self.column2
    }

    fn cell_count(&self) -> i64 {
        (self.row2 - self.row1 + 1) as i64 * (self.column2 - self.column1 + 1) as i64
    }

    fn cells(&self) -> impl Iterator<Item = CellKey> + '_ {
        (self.row1..=self.row2).flat_map(move |row| {
            (self.column1..=self.column2).map(move |column| (self.sheet, row, column))
        })
    }
}

/// What one run of a formula read.
#[derive(Clone, Default, PartialEq, Debug)]
pub(crate) struct Reads {
    pub(crate) cells: Vec<CellKey>,
    pub(crate) rects: Vec<Rect>,
    /// It read something that is not in a cell and that no write announces:
    /// the clock, a random number, the environment, the text of a formula.
    /// Such a formula runs in every evaluation.
    pub(crate) always: bool,
    /// It read where cells are, or worked out at run time which cells to
    /// read: `ROW()`, `OFFSET`, a name. Moving rows or columns can change its
    /// value without changing a cell it read, so it runs whenever they move.
    pub(crate) positional: bool,
}

/// Rows or columns of a sheet changed place: every one from `first` on moved
/// by `delta`. With a negative `delta` the ones they landed on are gone.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shift {
    pub(crate) sheet: u32,
    pub(crate) rows: bool,
    pub(crate) first: i32,
    pub(crate) delta: i32,
}

impl Shift {
    fn last(&self) -> i32 {
        if self.rows {
            LAST_ROW
        } else {
            LAST_COLUMN
        }
    }

    /// Where the cell that was at `key` is now, if it still exists.
    pub(crate) fn cell(&self, key: CellKey) -> Option<CellKey> {
        let (sheet, row, column) = key;
        if sheet != self.sheet {
            return Some(key);
        }
        let at = if self.rows { row } else { column };
        if at < self.first + self.delta.min(0) {
            return Some(key);
        }
        if at < self.first || at + self.delta > self.last() {
            return None;
        }
        Some(if self.rows {
            (sheet, row + self.delta, column)
        } else {
            (sheet, row, column + self.delta)
        })
    }

    /// Where a rectangle is now, if it holds the cells it held: all of it
    /// moved, or none of it. `None` if rows or columns were put into it, taken
    /// out of it, or moved within it.
    pub(crate) fn rect(&self, rect: &Rect) -> Option<Rect> {
        if rect.sheet != self.sheet {
            return Some(*rect);
        }
        let (from, to) = if self.rows {
            (rect.row1, rect.row2)
        } else {
            (rect.column1, rect.column2)
        };
        if to < self.first + self.delta.min(0) {
            return Some(*rect);
        }
        if from < self.first || to + self.delta > self.last() {
            return None;
        }
        let mut moved = *rect;
        if self.rows {
            moved.row1 += self.delta;
            moved.row2 += self.delta;
        } else {
            moved.column1 += self.delta;
            moved.column2 += self.delta;
        }
        Some(moved)
    }
}

/// What a position holds, as far as finding what to run goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Holds {
    NoFormula,
    Formula,
    /// The anchor of an array, and the area the array fills.
    Array(Rect),
}

/// Where a change was made: the starting points for finding what to run.
#[derive(Default, Debug)]
pub(crate) struct Seeds {
    /// Positions whose contents changed.
    pub(crate) cells: Vec<CellKey>,
    /// Rectangles in which anything may have changed.
    pub(crate) rects: Vec<Rect>,
    /// Formulas that must run whatever they read.
    pub(crate) formulas: Vec<CellKey>,
}

/// The place of a node in `Graph::nodes`.
type NodeId = u32;

/// The formulas that read one cell. Most cells are read by one formula, and
/// a list for each of those cost more than everything else kept here.
#[derive(Default)]
enum Readers {
    #[default]
    None,
    One(NodeId),
    Many(Vec<NodeId>),
}

impl Readers {
    fn as_slice(&self) -> &[NodeId] {
        match self {
            Readers::None => &[],
            Readers::One(reader) => std::slice::from_ref(reader),
            Readers::Many(readers) => readers,
        }
    }

    fn add(&mut self, formula: NodeId) {
        // A formula that reads a cell twice reads it twice in a row more
        // often than not.
        match self {
            Readers::None => *self = Readers::One(formula),
            Readers::One(reader) if *reader == formula => {}
            Readers::One(reader) => *self = Readers::Many(vec![*reader, formula]),
            Readers::Many(readers) => {
                if readers.last() != Some(&formula) {
                    readers.push(formula);
                }
            }
        }
    }

    fn remove(&mut self, formula: NodeId) {
        match self {
            Readers::None => {}
            Readers::One(reader) => {
                if *reader == formula {
                    *self = Readers::None;
                }
            }
            Readers::Many(readers) => {
                readers.retain(|reader| *reader != formula);
                if readers.is_empty() {
                    *self = Readers::None;
                }
            }
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, Readers::None)
    }
}

/// What is kept about one position.
#[derive(Default)]
struct Node {
    position: CellKey,
    /// What the formula here read when it last ran. Empty if none did.
    reads: Reads,
    /// Whether a formula ran here: whether `reads` is a record.
    ran: bool,
    /// Whether the formula that ran here is the anchor of an array.
    array: bool,
    /// The formulas that read this cell, as a cell and not as part of a
    /// rectangle.
    readers: Readers,
    /// Whether the node is out of use, waiting to be one again.
    spare: bool,
}

/// How many rows of a column have their nodes found together.
const ROWS_TOGETHER: i32 = 64;

const NOWHERE: NodeId = NodeId::MAX;

/// Where the node of a position is. The rows of a column are taken sixty-four
/// at a time: a block of them is found by a search, and a row in the block
/// by its number. Whatever goes down a column searches the same few blocks
/// over and over, which is as good as not searching.
#[derive(Default)]
struct Places {
    /// By `(sheet, column, row / ROWS_TOGETHER)`.
    blocks: HashMap<(u32, i32, i32), Box<[NodeId; ROWS_TOGETHER as usize]>, KeyBuild>,
}

impl Places {
    fn block((sheet, row, column): CellKey) -> ((u32, i32, i32), usize) {
        (
            (sheet, column, row.div_euclid(ROWS_TOGETHER)),
            row.rem_euclid(ROWS_TOGETHER) as usize,
        )
    }

    fn get(&self, position: CellKey) -> Option<NodeId> {
        let (block, row) = Places::block(position);
        let node = self.blocks.get(&block)?[row];
        (node != NOWHERE).then_some(node)
    }

    fn set(&mut self, position: CellKey, node: NodeId) {
        let (block, row) = Places::block(position);
        self.blocks
            .entry(block)
            .or_insert_with(|| Box::new([NOWHERE; ROWS_TOGETHER as usize]))[row] = node;
    }

    fn remove(&mut self, position: CellKey) {
        let (block, row) = Places::block(position);
        if let Some(rows) = self.blocks.get_mut(&block) {
            rows[row] = NOWHERE;
        }
    }
}

/// What `Graph::displace` found.
#[derive(Default)]
pub(crate) struct Displaced {
    pub(crate) cut_off: Vec<CellKey>,
    pub(crate) moved: Vec<CellKey>,
}

/// A rectangle at most this many columns wide is filed under each of its
/// columns; a wider one is looked at for every position.
const NARROW: i32 = 64;

/// A rectangle at most this many rows high is found by its first row; a
/// higher one is looked at for every position of its columns.
const SHORT: i32 = 256;

/// The rectangles filed under one column of a sheet, each with the formula
/// that read it.
#[derive(Default)]
struct Filed {
    /// Those at most `SHORT` rows high, sorted by their first row: the ones
    /// that hold a row start at most `SHORT` rows above it.
    short: Vec<(Rect, NodeId)>,
    tall: Vec<(Rect, NodeId)>,
}

impl Filed {
    /// The short rectangles that can reach into the rows `row1..=row2`.
    fn short_near(&self, row1: i32, row2: i32) -> &[(Rect, NodeId)] {
        let from = self
            .short
            .partition_point(|(rect, _)| rect.row1 <= row1 - SHORT);
        let to = self.short.partition_point(|(rect, _)| rect.row1 <= row2);
        &self.short[from..to]
    }
}

/// The lists of rectangles that have nothing left to give while a scope is
/// being grown: every formula in them is in the scope already. A column read
/// whole by ten thousand formulas is gone through once, not once for every
/// cell of it that changed.
#[derive(Default)]
struct Spent {
    tall: HashSet<(u32, i32), KeyBuild>,
    wide: bool,
}

/// The way back from a position to the formulas that read a rectangle it is
/// in. Filed again from what the formulas read whenever that changed, and
/// only when it is asked.
#[derive(Default)]
struct RectReaders {
    /// The narrow rectangles, under `(sheet, column)` for each of their
    /// columns.
    narrow: HashMap<(u32, i32), Filed, KeyBuild>,
    wide: Vec<(Rect, NodeId)>,
    /// The last column a narrow rectangle is filed under, in any sheet.
    last_filed_column: i32,
    stale: bool,
}

impl RectReaders {
    /// Brings the files in line with what the formulas of `nodes` read, if
    /// one of them read other rectangles since they were made.
    fn file(&mut self, nodes: &[Node]) {
        if !self.stale {
            return;
        }
        self.narrow.clear();
        self.wide.clear();
        self.last_filed_column = 0;
        for (formula, node) in nodes.iter().enumerate() {
            let formula = formula as NodeId;
            for rect in &node.reads.rects {
                if rect.column2 - rect.column1 >= NARROW {
                    self.wide.push((*rect, formula));
                    continue;
                }
                self.last_filed_column = self.last_filed_column.max(rect.column2);
                for column in rect.column1..=rect.column2 {
                    let filed = self.narrow.entry((rect.sheet, column)).or_default();
                    if rect.row2 - rect.row1 < SHORT {
                        filed.short.push((*rect, formula));
                    } else {
                        filed.tall.push((*rect, formula));
                    }
                }
            }
        }
        for filed in self.narrow.values_mut() {
            filed.short.sort_unstable_by_key(|(rect, _)| rect.row1);
        }
        self.stale = false;
    }

    /// Adds to `found` the formulas that read a rectangle with a cell in
    /// `area`. `in_scope` says whether a formula is in the scope being grown.
    fn of(
        &self,
        area: &Rect,
        in_scope: impl Fn(NodeId) -> bool,
        spent: &mut Spent,
        found: &mut Vec<NodeId>,
    ) {
        // Goes through a list no row tells apart, and says whether it is
        // spent: whether every formula in it is in the scope or on its way.
        let all = |list: &[(Rect, NodeId)], found: &mut Vec<NodeId>| {
            let mut spent = true;
            for (rect, reader) in list {
                if rect.intersects(area) {
                    found.push(*reader);
                } else if !in_scope(*reader) {
                    spent = false;
                }
            }
            spent
        };
        if self.narrow.is_empty() && self.wide.is_empty() {
            return;
        }
        for column in area.column1..=area.column2 {
            let Some(filed) = self.narrow.get(&(area.sheet, column)) else {
                // Past the columns anything was filed under there is nothing.
                if column > self.last_filed_column {
                    break;
                }
                continue;
            };
            for (rect, reader) in filed.short_near(area.row1, area.row2) {
                if rect.intersects(area) {
                    found.push(*reader);
                }
            }
            if !spent.tall.contains(&(area.sheet, column)) && all(&filed.tall, found) {
                spent.tall.insert((area.sheet, column));
            }
        }
        if !spent.wide && all(&self.wide, found) {
            spent.wide = true;
        }
    }
}

/// The dependency graph: for every formula, what it read when it last ran,
/// and the way back, from a position to the formulas that read it.
///
/// What a formula read, `Node::reads`, is the record. The rest says the same
/// the other way around and is kept in step with it by `link` and `unlink`.
#[derive(Default)]
pub(crate) struct Graph {
    nodes: Vec<Node>,
    /// For every node, the mark of the last scope it was put in. Apart from
    /// the nodes: an evaluation asks of every formula it meets whether it is
    /// in the scope, and this is all it needs to look at.
    marks: Vec<u32>,
    places: Places,
    /// The nodes out of use.
    spare: Vec<NodeId>,
    rect_readers: RectReaders,
    always: HashSet<NodeId, KeyBuild>,
    positional: HashSet<NodeId, KeyBuild>,
    /// The mark of the last scope started.
    mark: u32,
}

impl Graph {
    /// The node of a position, made if there is none.
    fn node_at(&mut self, position: CellKey) -> NodeId {
        if let Some(node) = self.places.get(position) {
            return node;
        }
        let node = match self.spare.pop() {
            Some(node) => node,
            None => {
                self.nodes.push(Node::default());
                self.marks.push(0);
                (self.nodes.len() - 1) as NodeId
            }
        };
        self.marks[node as usize] = 0;
        self.nodes[node as usize] = Node {
            position,
            ..Node::default()
        };
        self.places.set(position, node);
        node
    }

    /// Puts a node out of use if nothing is kept in it any more: no formula
    /// ran there, no formula reads it, and it is not waiting to run, as a
    /// formula just typed in is.
    fn retire(&mut self, node: NodeId) {
        let waiting = self.marks[node as usize] == self.mark;
        let kept = &mut self.nodes[node as usize];
        if kept.ran || !kept.readers.is_empty() || waiting || kept.spare {
            return;
        }
        self.places.remove(kept.position);
        *kept = Node {
            spare: true,
            ..Node::default()
        };
        self.spare.push(node);
    }

    /// Puts what `formula` read in place of what it read before. If that is
    /// what it read before, as it mostly is, nothing changes and `reads` is
    /// handed back, to be used again.
    /// `array` says whether the formula is the anchor of an array.
    pub(crate) fn set_reads(
        &mut self,
        formula: CellKey,
        reads: Reads,
        array: bool,
    ) -> Option<Reads> {
        let node = self.node_at(formula);
        let kept = &mut self.nodes[node as usize];
        kept.array = array;
        if kept.ran && kept.reads == reads {
            return Some(reads);
        }
        let before = std::mem::replace(&mut kept.reads, reads);
        kept.ran = true;
        self.unlink(node, &before);
        self.link(node);
        None
    }

    /// Makes room for what this many formulas are about to read.
    pub(crate) fn reserve(&mut self, formulas: usize) {
        let more = formulas.saturating_sub(self.nodes.len());
        self.nodes.reserve(more);
        self.marks.reserve(more);
    }

    /// Forgets the formula at a position: it is no longer one.
    pub(crate) fn remove(&mut self, formula: CellKey) {
        let Some(node) = self.places.get(formula) else {
            return;
        };
        let kept = &mut self.nodes[node as usize];
        let before = std::mem::take(&mut kept.reads);
        kept.ran = false;
        kept.array = false;
        self.unlink(node, &before);
        self.retire(node);
    }

    /// Enters the formula of a node under everything it read.
    fn link(&mut self, formula: NodeId) {
        let reads = std::mem::take(&mut self.nodes[formula as usize].reads);
        let position = self.nodes[formula as usize].position;
        for cell in &reads.cells {
            if *cell != position {
                let read = self.node_at(*cell);
                self.nodes[read as usize].readers.add(formula);
            }
        }
        if !reads.rects.is_empty() {
            self.rect_readers.stale = true;
        }
        if reads.always {
            self.always.insert(formula);
        }
        if reads.positional {
            self.positional.insert(formula);
        }
        self.nodes[formula as usize].reads = reads;
    }

    /// Undoes `link`, given what the formula had read.
    fn unlink(&mut self, formula: NodeId, reads: &Reads) {
        for cell in &reads.cells {
            if let Some(read) = self.places.get(*cell) {
                self.nodes[read as usize].readers.remove(formula);
                self.retire(read);
            }
        }
        if !reads.rects.is_empty() {
            self.rect_readers.stale = true;
        }
        if reads.always {
            self.always.remove(&formula);
        }
        if reads.positional {
            self.positional.remove(&formula);
        }
    }

    /// The nodes a formula ran at.
    fn formulas(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter(|node| node.ran)
    }

    /// The formulas on record whose own position is in `rect`.
    pub(crate) fn formulas_in(&self, rect: &Rect) -> Vec<CellKey> {
        self.formulas()
            .map(|node| node.position)
            .filter(|formula| rect.contains(*formula))
            .collect()
    }

    /// Follows cells to where they moved. Returns, where they are now, the
    /// formulas that no longer read what they read, one that read a cell that
    /// is gone, or a rectangle that did not move whole, or that is positional:
    /// what is on record for those is not to be trusted until they run. And
    /// the formulas that changed place themselves.
    ///
    /// Nodes keep their place in the vector, so who reads whom stands as it
    /// is: only the positions written in the nodes follow the cells.
    pub(crate) fn displace(&mut self, shift: &Shift) -> Displaced {
        let mut displaced = Displaced::default();
        let mut gone = Vec::new();
        let mut moved = Vec::new();
        for (id, node) in self.nodes.iter_mut().enumerate() {
            if node.spare {
                continue;
            }
            let was = node.position;
            match shift.cell(was) {
                Some(now) if now == was => {}
                Some(now) => {
                    node.position = now;
                    moved.push((id as NodeId, was, now));
                    if node.ran {
                        displaced.moved.push(now);
                    }
                }
                None => {
                    gone.push((id as NodeId, was));
                    continue;
                }
            }
            if !node.ran {
                continue;
            }
            let mut cut = node.reads.positional;
            node.reads.cells.retain_mut(|cell| match shift.cell(*cell) {
                Some(now) => {
                    *cell = now;
                    true
                }
                None => {
                    cut = true;
                    false
                }
            });
            for rect in &mut node.reads.rects {
                match shift.rect(rect) {
                    Some(now) => {
                        if now != *rect {
                            *rect = now;
                            self.rect_readers.stale = true;
                        }
                    }
                    None => cut = true,
                }
            }
            if cut {
                displaced.cut_off.push(node.position);
            }
        }
        // The places are emptied before any is filled: a cell moves to where
        // another was.
        for (_, was) in &gone {
            self.places.remove(*was);
        }
        for (_, was, _) in &moved {
            self.places.remove(*was);
        }
        for (node, _, now) in moved {
            self.places.set(now, node);
        }
        for (node, _) in gone {
            // What a formula there read, it no longer reads. That was put on
            // record before the move: it is looked for where it is now.
            let mut reads = std::mem::take(&mut self.nodes[node as usize].reads);
            reads.cells.retain_mut(|cell| match shift.cell(*cell) {
                Some(now) => {
                    *cell = now;
                    true
                }
                None => false,
            });
            self.unlink(node, &reads);
            self.nodes[node as usize] = Node {
                spare: true,
                ..Node::default()
            };
            self.spare.push(node);
        }
        displaced
    }

    /// Starts a scope: no formula is in it yet.
    pub(crate) fn new_scope(&mut self) -> Scope {
        self.mark += 1;
        Scope {
            formulas: Vec::new(),
            mark: self.mark,
        }
    }

    /// Whether the formula at a position is in `scope`.
    pub(crate) fn in_scope(&self, scope: &Scope, formula: CellKey) -> bool {
        self.places
            .get(formula)
            .is_some_and(|node| self.marks[node as usize] == scope.mark)
    }

    /// Marks the formulas of `scope` again, after another scope was grown.
    #[cfg(feature = "recalc_verify")]
    pub(crate) fn restore(&mut self, scope: &Scope) {
        for formula in &scope.formulas {
            if let Some(node) = self.places.get(*formula) {
                self.marks[node as usize] = scope.mark;
            }
        }
    }

    /// Adds to `scope` the formulas that must run given `seeds`: the formulas
    /// named there, those that read a position that changed, those that read
    /// one of those, and so on; and with every array among them, those that
    /// read a cell of the area it fills. `holds` says what a position holds
    /// now; an array that has not run yet has its area among the seeds.
    /// Returns whether the scope grew.
    pub(crate) fn grow(
        &mut self,
        scope: &mut Scope,
        seeds: Seeds,
        holds: impl Fn(CellKey) -> Holds,
    ) -> bool {
        self.rect_readers.file(&self.nodes);
        let before = scope.len();
        let mark = scope.mark;
        let Seeds {
            cells,
            mut rects,
            formulas,
        } = seeds;
        // The formulas to put in the scope, and the formulas put in it whose
        // readers are still to be found.
        let mut found: Vec<NodeId> = Vec::new();
        let mut unread: Vec<NodeId> = Vec::new();
        let mut spent = Spent::default();
        for formula in formulas {
            found.push(self.node_at(formula));
        }
        rects.extend(cells.into_iter().map(|cell| Rect::sized(cell, (1, 1))));
        loop {
            for formula in found.drain(..) {
                if self.marks[formula as usize] != mark {
                    self.marks[formula as usize] = mark;
                    let node = &self.nodes[formula as usize];
                    scope.formulas.push(node.position);
                    unread.push(formula);
                    // Only an array is looked up in its sheet, for its area.
                    if node.array {
                        if let Holds::Array(area) = holds(node.position) {
                            rects.push(area);
                        }
                    }
                }
            }
            let (nodes, marks) = (&self.nodes, &self.marks);
            let in_scope = |formula: NodeId| marks[formula as usize] == mark;
            if let Some(formula) = unread.pop() {
                let node = &nodes[formula as usize];
                found.extend_from_slice(node.readers.as_slice());
                let area = Rect::sized(node.position, (1, 1));
                self.rect_readers
                    .of(&area, in_scope, &mut spent, &mut found);
            } else if let Some(area) = rects.pop() {
                self.readers_of_cells(&area, &mut found);
                self.rect_readers
                    .of(&area, in_scope, &mut spent, &mut found);
            } else {
                break;
            }
        }
        scope.len() > before
    }

    /// The formulas that run in every evaluation.
    pub(crate) fn always(&self) -> Vec<CellKey> {
        self.always
            .iter()
            .map(|formula| self.nodes[*formula as usize].position)
            .collect()
    }

    /// The formulas that read, as a cell, a cell of `area`.
    fn readers_of_cells(&self, area: &Rect, found: &mut Vec<NodeId>) {
        if area.cell_count() <= self.nodes.len() as i64 {
            for cell in area.cells() {
                if let Some(node) = self.places.get(cell) {
                    found.extend_from_slice(self.nodes[node as usize].readers.as_slice());
                }
            }
        } else {
            for node in &self.nodes {
                if area.contains(node.position) {
                    found.extend_from_slice(node.readers.as_slice());
                }
            }
        }
    }

    /// What the formula at a position read when it last ran, if one ran.
    #[cfg(feature = "recalc_verify")]
    fn on_record(&self, formula: CellKey) -> Option<&Reads> {
        let node = &self.nodes[self.places.get(formula)? as usize];
        node.ran.then_some(&node.reads)
    }

    /// Says how this graph differs from `other`, if it does. For tests: a
    /// graph kept up to date edit by edit must be the graph a full evaluation
    /// builds from nothing. What the formulas of `unsteady` read is left out:
    /// they read the clock or random numbers, and what they read can depend
    /// on it.
    #[cfg(feature = "recalc_verify")]
    pub(crate) fn difference(
        &self,
        other: &Graph,
        unsteady: &HashSet<CellKey, KeyBuild>,
    ) -> Option<String> {
        fn sorted(reads: &Reads) -> Reads {
            let mut reads = reads.clone();
            reads.cells.sort_unstable();
            reads.cells.dedup();
            reads.rects.sort_unstable();
            reads.rects.dedup();
            reads
        }
        for node in self.formulas() {
            let formula = node.position;
            if unsteady.contains(&formula) {
                continue;
            }
            match other.on_record(formula) {
                None => return Some(format!("{formula:?} is on record and should not be")),
                // A formula that runs every time is in every scope, whatever
                // it read. One found circular before it ran is such a formula
                // and read nothing; found circular as it ran, it read.
                Some(expected) if expected.always || node.reads.always => {}
                Some(expected) if sorted(expected) != sorted(&node.reads) => {
                    return Some(format!(
                        "{formula:?} has {:?} on record, a full evaluation {:?}",
                        sorted(&node.reads),
                        sorted(expected)
                    ));
                }
                Some(_) => {}
            }
        }
        other
            .formulas()
            .map(|node| node.position)
            .find(|formula| self.on_record(*formula).is_none())
            .map(|formula| format!("{formula:?} is not on record"))
    }
}
