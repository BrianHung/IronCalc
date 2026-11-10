//! What every formula read the last time it ran, and who reads a position.

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

/// A rectangle at most this many columns wide is filed under each of its
/// columns; a wider one is looked at for every position.
const NARROW: i32 = 64;

/// The dependency graph: for every formula, what it read when it last ran,
/// and the way back, from a position to the formulas that read it.
#[derive(Default)]
pub(crate) struct Graph {
    reads: HashMap<CellKey, Reads, KeyBuild>,
    readers: HashMap<CellKey, HashSet<CellKey, KeyBuild>, KeyBuild>,
    /// The rectangles read, filed by `(sheet, column)` when they are narrow,
    /// with the formula that read them. Built when it is needed, from `reads`.
    narrow: HashMap<(u32, i32), Vec<(Rect, CellKey)>, KeyBuild>,
    wide: Vec<(Rect, CellKey)>,
    rects_stale: bool,
    always: HashSet<CellKey, KeyBuild>,
    positional: HashSet<CellKey, KeyBuild>,
}

impl Graph {
    /// Puts what `formula` read in place of what it read before.
    pub(crate) fn set_reads(&mut self, formula: CellKey, reads: Reads) {
        if self.reads.get(&formula) == Some(&reads) {
            return;
        }
        self.remove(formula);
        self.insert(formula, reads);
    }

    fn insert(&mut self, formula: CellKey, reads: Reads) {
        for cell in &reads.cells {
            if *cell != formula {
                self.readers.entry(*cell).or_default().insert(formula);
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
                readers.remove(&formula);
                if readers.is_empty() {
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

    /// Follows cells to where they moved. Returns the formulas, where they
    /// are now, that no longer read what they read: one that read a cell that
    /// is gone, or a rectangle that did not move whole, or that is positional.
    /// What is on record for those is not to be trusted until they run.
    pub(crate) fn displace(&mut self, shift: &Shift) -> Vec<CellKey> {
        let old = std::mem::take(&mut self.reads);
        self.readers.clear();
        self.always.clear();
        self.positional.clear();
        self.rects_stale = true;
        let mut cut_off = Vec::new();
        for (formula, mut reads) in old {
            let Some(formula) = shift.cell(formula) else {
                continue;
            };
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
                cut_off.push(formula);
            }
            self.insert(formula, reads);
        }
        cut_off
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
                self.readers_of(cell, &mut found);
            } else if let Some(rect) = rects.pop() {
                self.readers_of_rect(&rect, &mut found);
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

    fn readers_of(&self, position: CellKey, found: &mut Vec<CellKey>) {
        if let Some(readers) = self.readers.get(&position) {
            found.extend(readers.iter().copied());
        }
        let (sheet, _, column) = position;
        let narrow = self.narrow.get(&(sheet, column)).into_iter().flatten();
        for (rect, reader) in narrow.chain(self.wide.iter()) {
            if rect.contains(position) {
                found.push(*reader);
            }
        }
    }

    fn readers_of_rect(&self, area: &Rect, found: &mut Vec<CellKey>) {
        if area.cell_count() <= self.readers.len() as i64 {
            for cell in area.cells() {
                if let Some(readers) = self.readers.get(&cell) {
                    found.extend(readers.iter().copied());
                }
            }
        } else {
            for (cell, readers) in &self.readers {
                if area.contains(*cell) {
                    found.extend(readers.iter().copied());
                }
            }
        }
        let narrow = self
            .narrow
            .iter()
            .filter(|((sheet, column), _)| {
                *sheet == area.sheet && (area.column1..=area.column2).contains(column)
            })
            .flat_map(|(_, rects)| rects.iter());
        for (rect, reader) in narrow.chain(self.wide.iter()) {
            if rect.intersects(area) {
                found.push(*reader);
            }
        }
    }

    fn file_rects(&mut self) {
        if !self.rects_stale {
            return;
        }
        self.narrow.clear();
        self.wide.clear();
        for (formula, reads) in &self.reads {
            for rect in &reads.rects {
                if rect.column2 - rect.column1 < NARROW {
                    for column in rect.column1..=rect.column2 {
                        self.narrow
                            .entry((rect.sheet, column))
                            .or_default()
                            .push((*rect, *formula));
                    }
                } else {
                    self.wide.push((*rect, *formula));
                }
            }
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
