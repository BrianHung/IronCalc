//! What every formula read the last time it ran, and who reads a position.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use crate::constants::{LAST_COLUMN, LAST_ROW};
use crate::evaluation::{CellKey, KeyBuild};
use crate::expressions::types::CellReferenceIndex;

/// The formulas an evaluation runs.
pub(crate) type Scope = HashSet<CellKey, KeyBuild>;

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

/// The formulas that read one cell. Most cells are read by one formula, and
/// a list for each of those cost more than everything else kept here.
enum Readers {
    One(CellKey),
    Many(Vec<CellKey>),
}

impl Readers {
    fn as_slice(&self) -> &[CellKey] {
        match self {
            Readers::One(reader) => std::slice::from_ref(reader),
            Readers::Many(readers) => readers,
        }
    }

    fn add(&mut self, formula: CellKey) {
        // A formula that reads a cell twice reads it twice in a row more
        // often than not.
        match self {
            Readers::One(reader) if *reader == formula => {}
            Readers::One(reader) => *self = Readers::Many(vec![*reader, formula]),
            Readers::Many(readers) => {
                if readers.last() != Some(&formula) {
                    readers.push(formula);
                }
            }
        }
    }

    /// Takes a formula out. Returns whether none is left.
    fn remove(&mut self, formula: CellKey) -> bool {
        match self {
            Readers::One(reader) => *reader == formula,
            Readers::Many(readers) => {
                readers.retain(|reader| *reader != formula);
                readers.is_empty()
            }
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
    short: Vec<(Rect, CellKey)>,
    tall: Vec<(Rect, CellKey)>,
}

impl Filed {
    /// The short rectangles that can reach into the rows `row1..=row2`.
    fn short_near(&self, row1: i32, row2: i32) -> &[(Rect, CellKey)] {
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

/// The dependency graph: for every formula, what it read when it last ran,
/// and the way back, from a position to the formulas that read it.
#[derive(Default)]
pub(crate) struct Graph {
    reads: HashMap<CellKey, Reads, KeyBuild>,
    /// The formulas that read a cell, as a cell and not as part of a
    /// rectangle.
    readers: HashMap<CellKey, Readers, KeyBuild>,
    /// The rectangles read, filed by `(sheet, column)` when they are narrow,
    /// with the formula that read them. Built when it is needed, from `reads`.
    narrow: HashMap<(u32, i32), Filed, KeyBuild>,
    wide: Vec<(Rect, CellKey)>,
    /// The last column a narrow rectangle is filed under, in any sheet.
    last_filed_column: i32,
    rects_stale: bool,
    always: HashSet<CellKey, KeyBuild>,
    positional: HashSet<CellKey, KeyBuild>,
}

impl Graph {
    /// Puts what `formula` read in place of what it read before. If that is
    /// what it read before, as it mostly is, nothing changes and `reads` is
    /// handed back, to be used again.
    pub(crate) fn set_reads(&mut self, formula: CellKey, reads: Reads) -> Option<Reads> {
        if self.reads.get(&formula) == Some(&reads) {
            return Some(reads);
        }
        self.remove(formula);
        self.insert(formula, reads);
        None
    }

    fn insert(&mut self, formula: CellKey, reads: Reads) {
        for cell in &reads.cells {
            if *cell != formula {
                match self.readers.entry(*cell) {
                    Entry::Occupied(mut readers) => readers.get_mut().add(formula),
                    Entry::Vacant(none) => {
                        none.insert(Readers::One(formula));
                    }
                }
            }
        }
        if !reads.rects.is_empty() {
            self.rects_stale = true;
        }
        if reads.always {
            self.always.insert(formula);
        }
        if reads.positional {
            self.positional.insert(formula);
        }
        self.reads.insert(formula, reads);
    }

    /// Forgets `formula`: it is no longer a formula, or it is about to read
    /// something else.
    pub(crate) fn remove(&mut self, formula: CellKey) {
        let Some(old) = self.reads.remove(&formula) else {
            return;
        };
        for cell in &old.cells {
            if let Some(readers) = self.readers.get_mut(cell) {
                if readers.remove(formula) {
                    self.readers.remove(cell);
                }
            }
        }
        if !old.rects.is_empty() {
            self.rects_stale = true;
        }
        self.always.remove(&formula);
        self.positional.remove(&formula);
    }

    /// The formulas on record whose own position is in `rect`.
    pub(crate) fn formulas_in(&self, rect: &Rect) -> Vec<CellKey> {
        self.reads
            .keys()
            .filter(|formula| rect.contains(**formula))
            .copied()
            .collect()
    }

    /// Follows cells to where they moved. Returns, where they are now, the
    /// formulas that no longer read what they read, one that read a cell that
    /// is gone, or a rectangle that did not move whole, or that is positional:
    /// what is on record for those is not to be trusted until they run. And
    /// the formulas that changed place themselves.
    pub(crate) fn displace(&mut self, shift: &Shift) -> Displaced {
        // Most formulas neither moved nor read anything that did, when rows
        // are put in near the end of a sheet: those are left where they are.
        let touched: Vec<CellKey> = self
            .reads
            .iter()
            .filter(|(formula, reads)| {
                reads.positional
                    || shift.cell(**formula) != Some(**formula)
                    || reads
                        .cells
                        .iter()
                        .any(|cell| shift.cell(*cell) != Some(*cell))
                    || reads
                        .rects
                        .iter()
                        .any(|rect| shift.rect(rect) != Some(*rect))
            })
            .map(|(formula, _)| *formula)
            .collect();
        let moved: Vec<(CellKey, Reads)> = if touched.len() > self.reads.len() / 2 {
            // Nearly everything is touched: taking it all out at once is
            // cheaper than taking it out formula by formula.
            self.readers.clear();
            self.always.clear();
            self.positional.clear();
            self.rects_stale = true;
            std::mem::take(&mut self.reads).into_iter().collect()
        } else {
            let mut moved = Vec::with_capacity(touched.len());
            for formula in touched {
                if let Some(reads) = self.reads.get(&formula).cloned() {
                    self.remove(formula);
                    moved.push((formula, reads));
                }
            }
            moved
        };
        self.reads.reserve(moved.len());
        let mut displaced = Displaced::default();
        for (formula, mut reads) in moved {
            let was = formula;
            let Some(formula) = shift.cell(formula) else {
                continue;
            };
            if formula != was {
                displaced.moved.push(formula);
            }
            let mut cut = reads.positional;
            for cell in &mut reads.cells {
                match shift.cell(*cell) {
                    Some(moved) => *cell = moved,
                    None => cut = true,
                }
            }
            for rect in &mut reads.rects {
                match shift.rect(rect) {
                    Some(moved) => *rect = moved,
                    None => cut = true,
                }
            }
            if cut {
                displaced.cut_off.push(formula);
            }
            self.insert(formula, reads);
        }
        displaced
    }

    /// Adds to `scope` the formulas that must run given `seeds`: the formulas
    /// named there, those that read a position that changed, those that read
    /// one of those, and so on; and with every array among them, those that
    /// read a cell of the area it fills. `holds` says what a position holds
    /// now. Returns whether the scope grew.
    pub(crate) fn grow(
        &mut self,
        scope: &mut Scope,
        seeds: Seeds,
        holds: impl Fn(CellKey) -> Holds,
    ) -> bool {
        self.file_rects();
        let before = scope.len();
        let Seeds {
            mut cells,
            mut rects,
            formulas,
        } = seeds;
        let mut found = formulas;
        let mut spent = Spent::default();
        loop {
            for formula in found.drain(..) {
                if scope.insert(formula) {
                    cells.push(formula);
                    if let Holds::Array(area) = holds(formula) {
                        rects.push(area);
                    }
                }
            }
            if let Some(cell) = cells.pop() {
                let area = Rect::sized(cell, (1, 1));
                if let Some(readers) = self.readers.get(&cell) {
                    found.extend_from_slice(readers.as_slice());
                }
                self.readers_of_rects(&area, scope, &mut spent, &mut found);
            } else if let Some(area) = rects.pop() {
                self.readers_of_cells(&area, &mut found);
                self.readers_of_rects(&area, scope, &mut spent, &mut found);
            } else {
                break;
            }
        }
        scope.len() > before
    }

    /// The formulas that run in every evaluation.
    pub(crate) fn always(&self) -> Vec<CellKey> {
        self.always.iter().copied().collect()
    }

    /// The formulas that read, as a cell, a cell of `area`.
    fn readers_of_cells(&self, area: &Rect, found: &mut Vec<CellKey>) {
        if area.cell_count() <= self.readers.len() as i64 {
            for cell in area.cells() {
                if let Some(readers) = self.readers.get(&cell) {
                    found.extend_from_slice(readers.as_slice());
                }
            }
        } else {
            for (cell, readers) in &self.readers {
                if area.contains(*cell) {
                    found.extend_from_slice(readers.as_slice());
                }
            }
        }
    }

    /// The formulas that read a rectangle with a cell in `area`.
    fn readers_of_rects(
        &self,
        area: &Rect,
        scope: &Scope,
        spent: &mut Spent,
        found: &mut Vec<CellKey>,
    ) {
        // Goes through a list no row tells apart, and says whether it is
        // spent: whether every formula in it is in the scope or on its way.
        let all = |list: &[(Rect, CellKey)], found: &mut Vec<CellKey>| {
            let mut spent = true;
            for (rect, reader) in list {
                if rect.intersects(area) {
                    found.push(*reader);
                } else if !scope.contains(reader) {
                    spent = false;
                }
            }
            spent
        };
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

    fn file_rects(&mut self) {
        if !self.rects_stale {
            return;
        }
        self.narrow.clear();
        self.wide.clear();
        self.last_filed_column = 0;
        for (formula, reads) in &self.reads {
            for rect in &reads.rects {
                if rect.column2 - rect.column1 >= NARROW {
                    self.wide.push((*rect, *formula));
                    continue;
                }
                self.last_filed_column = self.last_filed_column.max(rect.column2);
                for column in rect.column1..=rect.column2 {
                    let filed = self.narrow.entry((rect.sheet, column)).or_default();
                    if rect.row2 - rect.row1 < SHORT {
                        filed.short.push((*rect, *formula));
                    } else {
                        filed.tall.push((*rect, *formula));
                    }
                }
            }
        }
        for filed in self.narrow.values_mut() {
            filed.short.sort_unstable_by_key(|(rect, _)| rect.row1);
        }
        self.rects_stale = false;
    }

    /// Says how this graph differs from `other`, if it does. For tests: a
    /// graph kept up to date edit by edit must be the graph a full evaluation
    /// builds from nothing. What the formulas of `unsteady` read is left out:
    /// they read the clock or random numbers, and what they read can depend
    /// on it.
    #[cfg(feature = "recalc_verify")]
    pub(crate) fn difference(&self, other: &Graph, unsteady: &Scope) -> Option<String> {
        fn sorted(reads: &Reads) -> Reads {
            let mut reads = reads.clone();
            reads.cells.sort_unstable();
            reads.cells.dedup();
            reads.rects.sort_unstable();
            reads.rects.dedup();
            reads
        }
        for (formula, reads) in &self.reads {
            if unsteady.contains(formula) {
                continue;
            }
            match other.reads.get(formula) {
                None => return Some(format!("{formula:?} is on record and should not be")),
                // A formula that runs every time is in every scope, whatever
                // it read. One found circular before it ran is such a formula
                // and read nothing; found circular as it ran, it read.
                Some(expected) if expected.always || reads.always => {}
                Some(expected) if sorted(expected) != sorted(reads) => {
                    return Some(format!(
                        "{formula:?} has {:?} on record, a full evaluation {:?}",
                        sorted(reads),
                        sorted(expected)
                    ));
                }
                Some(_) => {}
            }
        }
        other
            .reads
            .keys()
            .find(|formula| !self.reads.contains_key(*formula))
            .map(|formula| format!("{formula:?} is not on record"))
    }
}
