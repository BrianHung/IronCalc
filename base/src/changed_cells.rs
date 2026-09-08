//! What counts as an observable change, and the delta the API reports.
//!
//! A reader that paints a workbook on screen needs to know which cells to
//! repaint after an edit. [`Model::take_changed_cells`] answers that, and this
//! module decides what the answer means: one comparison key says whether a
//! cell moved, and everything that cannot be expressed as a list of cells
//! reports [`ChangedSinceRead::Everything`] instead.
//!
//! Two sources feed the delta, and they are complementary rather than
//! redundant:
//!
//! * the write journal (see [`crate::recalc::journal`]) names the cells the
//!   *user* edited, and
//! * a snapshot diff around [`Model::evaluate`] names the cells whose value
//!   the *pass* moved.
//!
//! The journal is drained at the head of `evaluate`, before the snapshot is
//! taken, so an edited cell's new content is already in the "before" picture
//! and the diff never re-reports it. Each source covers exactly what the other
//! cannot see.

use std::collections::{HashMap, HashSet};

use crate::cell::CellValue;
use crate::expressions::types::CellReferenceIndex;
use crate::model::Model;
use crate::recalc::Position;
use crate::types::{CellType, Link};

/// What cells changed since the last [`Model::take_changed_cells`].
///
/// A model starts `Untracked` and stays there until someone asks for the
/// delta. That is what makes the feature free for callers who never use it:
/// every hook below is one discriminant test away from doing nothing.
pub(crate) enum ChangedCells {
    /// Nobody has read the delta yet, so nothing is being recorded.
    Untracked,
    /// Tracking, and the next read is a full repaint: something changed that
    /// a list of cells cannot describe.
    All,
    /// Tracking; these cells moved since the last read. Possibly empty.
    Delta(HashSet<Position>),
}

/// Cells that changed since the last [`Model::take_changed_cells`].
///
/// `Everything` means "rescan the workbook"; `Cells` is the delta, possibly
/// empty. An empty delta and a full repaint are not the same answer, so this
/// is not an `Option<Vec<_>>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangedSinceRead {
    /// Rescan the workbook. See [`Model::take_changed_cells`] for the list of
    /// changes that report this.
    Everything,
    /// The cells whose content moved since the last read, in
    /// `(sheet, row, column)` order.
    Cells(Vec<CellReferenceIndex>),
}

/// A cell's observable content, as the delta compares it: its value, its type
/// (so a `#VALUE!` and a cell holding that same text differ) and its dynamic
/// link (a `HYPERLINK` target can move under an unchanged label).
///
/// Static links -- the ones [`Model::set_cell_link`] writes -- are deliberately
/// not here: they only change through a journaled write, so the journal already
/// reports them and repeating the lookup here would buy nothing.
pub(crate) type ChangeKey = (CellType, ChangeValue, Option<Link>);

/// A cell value flattened for comparison. Numbers are kept as bits, so a
/// `+0.0`/`-0.0` flip counts as a change and a `NaN` does not report itself as
/// changed on every pass forever.
#[derive(PartialEq, Debug)]
pub(crate) enum ChangeValue {
    None,
    Boolean(bool),
    Number(u64),
    String(String),
}

/// Whether `key` is the state of a position with no cell.
///
/// [`Model::get_cell_value_by_index`] returns `None` at an empty position and
/// [`Model::get_cell_type`] returns `Number`, so a stored blank cell that only
/// carries a style has the same signature as a missing one. Nothing observable
/// separates them, so the snapshot leaves both out and the diff never reports
/// one turning into the other. This is the *only* definition of "absent": the
/// snapshot and the diff both read it from here.
fn is_absent(key: &ChangeKey) -> bool {
    matches!(key, (CellType::Number, ChangeValue::None, None))
}

impl Model<'_> {
    /// A cell's observable content. See [`ChangeKey`].
    fn change_key(&self, position @ (sheet, row, column): Position) -> Option<ChangeKey> {
        let value = match self.get_cell_value_by_index(sheet, row, column).ok()? {
            CellValue::None => ChangeValue::None,
            CellValue::Boolean(b) => ChangeValue::Boolean(b),
            // By bits, so a +0.0/-0.0 flip is seen and a NaN does not report
            // itself changed forever.
            CellValue::Number(n) => ChangeValue::Number(n.to_bits()),
            CellValue::String(s) => ChangeValue::String(s),
        };
        let cell_type = self.get_cell_type(sheet, row, column).ok()?;
        Some((cell_type, value, self.links.get(&position).cloned()))
    }

    /// The observable content of every position that has any, to diff a later
    /// snapshot against.
    ///
    /// Positions whose key [`is_absent`] are left out, so "missing from the
    /// map" and "empty" are one state rather than two. A dynamic link can be
    /// the only observable thing at a position, so link positions join the
    /// stored cells.
    fn workbook_change_keys(&self) -> HashMap<Position, ChangeKey> {
        let mut keys = HashMap::new();
        let record = |keys: &mut HashMap<Position, ChangeKey>, position: Position| {
            if let Some(key) = self.change_key(position) {
                if !is_absent(&key) {
                    keys.insert(position, key);
                }
            }
        };
        for (index, worksheet) in self.workbook.worksheets.iter().enumerate() {
            for (&row, row_data) in &worksheet.sheet_data {
                for &column in row_data.keys() {
                    record(&mut keys, (index as u32, row, column));
                }
            }
        }
        for &position in self.links.keys() {
            record(&mut keys, position);
        }
        keys
    }

    /// The snapshot [`Model::record_evaluated_changes`] will diff against, or
    /// `None` when the pass has nothing to report to.
    ///
    /// `None` while untracked -- which is what keeps an unsubscribed pass free
    /// -- and also once the delta is already `All`, because a full repaint
    /// cannot be made more complete by adding cells to it.
    pub(crate) fn change_keys_before_pass(&self) -> Option<HashMap<Position, ChangeKey>> {
        match self.changed_cells {
            ChangedCells::Delta(_) => Some(self.workbook_change_keys()),
            ChangedCells::Untracked | ChangedCells::All => None,
        }
    }

    /// Adds to the delta every cell the pass moved, by diffing the current
    /// state against the snapshot taken before it.
    pub(crate) fn record_evaluated_changes(&mut self, before: &HashMap<Position, ChangeKey>) {
        let after = self.workbook_change_keys();
        let ChangedCells::Delta(delta) = &mut self.changed_cells else {
            return;
        };
        for (position, key) in &after {
            if before.get(position) != Some(key) {
                delta.insert(*position);
            }
        }
        for position in before.keys() {
            if !after.contains_key(position) {
                delta.insert(*position);
            }
        }
    }

    /// Records that the next read must be a full repaint.
    ///
    /// Called by the changes that no list of cells can describe: a structural
    /// insert, delete or move (which renumbers cells rather than editing
    /// them), sheet creation, deletion, renaming and reordering, defined-name
    /// changes, and a locale, timezone or language switch (which re-renders
    /// values that never moved).
    ///
    /// A no-op while untracked: nothing is being recorded, and the first read
    /// is `Everything` regardless.
    ///
    /// Callers mark *before* validating their arguments, not after. An
    /// operation that then fails has changed nothing, so the cost of marking
    /// early is at worst one redundant repaint; marking after every early
    /// return is a standing invitation to lose a change the next time someone
    /// adds one. The safe direction is over-reporting.
    pub(crate) fn record_everything_changed(&mut self) {
        if !matches!(self.changed_cells, ChangedCells::Untracked) {
            self.changed_cells = ChangedCells::All;
        }
    }

    /// The cells whose content moved since the last call, sorted, clearing the
    /// record as it reads.
    ///
    /// The first call on a model returns [`ChangedSinceRead::Everything`] and
    /// starts tracking: asking for the delta is what turns the feature on, and
    /// a reader that has not read before has painted nothing, so the whole
    /// workbook is what it needs. A model whose delta is never read records
    /// nothing and pays nothing.
    ///
    /// Later calls return [`ChangedSinceRead::Cells`], except after a change
    /// no list of cells can describe -- see
    /// [`record_everything_changed`](Model::record_everything_changed).
    ///
    /// # What counts as a change
    ///
    /// A cell's *content*: its value, its type, and its `HYPERLINK` target.
    /// Both user edits and the values a recomputation moved are reported,
    /// whether or not [`Model::evaluate`] has run since the edit.
    ///
    /// Deliberately not reported, because none of it is cell content:
    /// styling (including conditional-format results and named-style edits),
    /// row heights and column widths, row and column visibility, and sheet
    /// metadata such as tab colour. A reader that paints those needs its own
    /// signal. The one apparent exception is a quote prefix, which is reported
    /// -- it is stored as a style flag, but it changes the cell's content as
    /// [`Model::get_localized_cell_content`] reports it.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::{ChangedSinceRead, Model};
    /// # use ironcalc_base::expressions::types::CellReferenceIndex;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// model.set_user_input(0, 1, 1, "1".to_string())?;
    /// model.set_user_input(0, 1, 2, "=A1*2".to_string())?;
    /// model.evaluate();
    ///
    /// // The first read is always a full repaint.
    /// assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);
    ///
    /// // A1 is the edit; B1 is the value the pass moved.
    /// model.set_user_input(0, 1, 1, "5".to_string())?;
    /// model.evaluate();
    /// assert_eq!(
    ///     model.take_changed_cells(),
    ///     ChangedSinceRead::Cells(vec![
    ///         CellReferenceIndex { sheet: 0, row: 1, column: 1 },
    ///         CellReferenceIndex { sheet: 0, row: 1, column: 2 },
    ///     ])
    /// );
    ///
    /// // Reading clears the record.
    /// assert_eq!(model.take_changed_cells(), ChangedSinceRead::Cells(vec![]));
    /// # Ok(())
    /// # }
    /// ```
    pub fn take_changed_cells(&mut self) -> ChangedSinceRead {
        // Edits made since the last pass are reported even if nothing has been
        // evaluated since.
        self.drain_write_journal();
        // Reading re-arms tracking: `Untracked` becomes an empty delta, which
        // is also how a first read starts recording.
        let taken = std::mem::replace(&mut self.changed_cells, ChangedCells::Delta(HashSet::new()));
        let ChangedCells::Delta(cells) = taken else {
            return ChangedSinceRead::Everything;
        };
        let mut cells: Vec<Position> = cells.into_iter().collect();
        cells.sort_unstable();
        ChangedSinceRead::Cells(
            cells
                .into_iter()
                .map(|(sheet, row, column)| CellReferenceIndex { sheet, row, column })
                .collect(),
        )
    }
}
