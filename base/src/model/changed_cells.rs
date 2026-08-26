//! What counts as an observable change, and the delta the API reports.
//!
//! The incremental pass propagates only where a recomputed cell actually moved,
//! and reports only the cells that moved. Both rest on one comparison key, so
//! "changed" means the same thing to the cutoff, to the delta, and to the
//! `Verify` oracle that checks them.

use std::collections::{HashMap, HashSet};

use crate::cell::CellValue;
use crate::cf_types::CfCellResult;
use crate::dependency_graph::Position;
use crate::expressions::types::CellReferenceIndex;
use crate::model::Model;
use crate::types::CellType;

/// What cells changed since the last [`Model::take_changed_cells`], backing the
/// incremental delta API.
pub(crate) enum ChangedCells {
    /// A full recompute ran: the next `take_changed_cells` is `Everything`,
    /// not an empty `Cells` list.
    All,
    /// Cells whose observable state moved on an incremental pass since the last
    /// read (not every cell that ran).
    Delta(HashSet<Position>),
}

/// Cells that changed since the last [`Model::take_changed_cells`].
///
/// `Everything` is a full pass (rescan the workbook). `Cells` is the incremental
/// delta, possibly empty. These are not the same kind of answer, so this is not
/// an `Option`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangedSinceRead {
    /// A full pass ran, or an insert/delete moved cells the dirty cone cannot
    /// name. Rescan the workbook.
    Everything,
    /// The incremental delta since the last read, possibly empty.
    Cells(Vec<CellReferenceIndex>),
}

/// A cell's observable signature for incremental change detection: value, type
/// (so an error and a same-text literal differ), and dynamic link.
pub(super) type ChangeKey = (CellType, ChangeValue, Option<crate::types::Link>);

/// A cell value flattened for change detection. Numbers are kept as bits so a
/// `+0.0`/`-0.0` flip is seen and a `NaN` does not report as changed forever.
#[derive(PartialEq, Debug)]
pub(super) enum ChangeValue {
    None,
    Boolean(bool),
    Number(u64),
    String(String),
}

impl Model<'_> {
    /// A cell's observable signature: value, type (so an error and a same-text
    /// literal differ), and dynamic link (a HYPERLINK target can move under a
    /// fixed label).
    pub(super) fn change_key(
        &self,
        position @ (sheet, row, column): Position,
    ) -> Option<ChangeKey> {
        let value = match self.get_cell_value_by_index(sheet, row, column).ok()? {
            CellValue::None => ChangeValue::None,
            CellValue::Boolean(b) => ChangeValue::Boolean(b),
            // By bits, so a +0.0/-0.0 flip is seen and NaN does not report forever.
            CellValue::Number(n) => ChangeValue::Number(n.to_bits()),
            CellValue::String(s) => ChangeValue::String(s),
        };
        let cell_type = self.get_cell_type(sheet, row, column).ok()?;
        Some((cell_type, value, self.links.get(&position).cloned()))
    }

    /// Adds to the delta any cell whose conditional-format result moved between
    /// `cf_before` and the rebuilt `cf_cache`. CF has no dependency edges, so a
    /// value or CF-rule change can move a cell's format with no value change.
    pub(super) fn record_cf_changes(&mut self, cf_before: HashMap<Position, Vec<CfCellResult>>) {
        if let ChangedCells::Delta(delta) = &mut self.changed_cells {
            for (position, results) in &self.cf_cache {
                if cf_before.get(position) != Some(results) {
                    delta.insert(*position);
                }
            }
            for position in cf_before.keys() {
                if !self.cf_cache.contains_key(position) {
                    delta.insert(*position);
                }
            }
        }
    }

    /// Returns the cells whose observable state moved on incremental evaluations
    /// since the last call, sorted, and clears the record. `Everything` means a
    /// full recompute has run, or an insert/delete moved cells the dirty cone
    /// cannot name. An empty `Cells` delta is not `Everything`.
    pub fn take_changed_cells(&mut self) -> ChangedSinceRead {
        self.drain_write_journal();
        // Reading re-arms tracking: the record resets to an empty delta, so
        // subsequent incremental passes accumulate afresh.
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
