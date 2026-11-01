//! What every formula read the last time it ran, and who reads a position.

use std::collections::{HashMap, HashSet};

use crate::evaluation::{CellKey, KeyBuild};
use crate::expressions::types::CellReferenceIndex;

/// A rectangle of one sheet, both corners included.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

    pub(crate) fn contains(&self, (sheet, row, column): CellKey) -> bool {
        sheet == self.sheet
            && (self.row1..=self.row2).contains(&row)
            && (self.column1..=self.column2).contains(&column)
    }
}

/// What one run of a formula read.
#[derive(Clone, Default, PartialEq, Debug)]
pub(crate) struct Reads {
    pub(crate) cells: Vec<CellKey>,
    pub(crate) rects: Vec<Rect>,
    /// It read something that is not in a cell and that no write announces:
    /// the clock, a random number, the environment, the text of a formula.
    /// Such a formula runs in every pass.
    pub(crate) beyond_cells: bool,
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
    beyond_cells: HashSet<CellKey, KeyBuild>,
}

impl Graph {
    /// Puts what `formula` read in place of what it read before.
    pub(crate) fn set_reads(&mut self, formula: CellKey, reads: Reads) {
        if self.reads.get(&formula) == Some(&reads) {
            return;
        }
        self.remove(formula);
        for cell in &reads.cells {
            if *cell != formula {
                self.readers.entry(*cell).or_default().insert(formula);
            }
        }
        if !reads.rects.is_empty() {
            self.rects_stale = true;
        }
        if reads.beyond_cells {
            self.beyond_cells.insert(formula);
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
        self.beyond_cells.remove(&formula);
    }

    pub(crate) fn formula_count(&self) -> usize {
        self.reads.len()
    }

    /// The formulas that must run when the cells at `written` changed: those
    /// that read one of them, those that read one of those, and so on, and
    /// those that read beyond cells. Stops and returns `None` as soon as there
    /// are more than `limit` of them.
    pub(crate) fn scope(
        &mut self,
        written: &[CellKey],
        limit: usize,
    ) -> Option<HashSet<CellKey, KeyBuild>> {
        self.file_rects();
        let mut scope: HashSet<CellKey, KeyBuild> = self.beyond_cells.clone();
        let mut pending: Vec<CellKey> = written.to_vec();
        pending.extend(scope.iter().copied());
        // A written position that holds a formula is in the scope itself:
        // the caller puts it there, since only the sheet knows.
        while let Some(position) = pending.pop() {
            if scope.len() > limit {
                return None;
            }
            for reader in self.readers_of(position) {
                if scope.insert(reader) {
                    pending.push(reader);
                }
            }
        }
        if scope.len() > limit {
            return None;
        }
        Some(scope)
    }

    fn readers_of(&self, position: CellKey) -> Vec<CellKey> {
        let mut found: Vec<CellKey> = self
            .readers
            .get(&position)
            .map(|readers| readers.iter().copied().collect())
            .unwrap_or_default();
        let (sheet, _, column) = position;
        let narrow = self.narrow.get(&(sheet, column)).into_iter().flatten();
        for (rect, reader) in narrow.chain(self.wide.iter()) {
            if *reader != position && rect.contains(position) {
                found.push(*reader);
            }
        }
        found
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
}
