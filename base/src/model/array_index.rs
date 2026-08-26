//! The array/spill footprint index, and the formula count that travels with it.
//!
//! An array anchor writes its members as evaluation writes, not edits, so
//! nothing journals them. The index maps every footprint position to the anchor
//! that produces it, which is how reading a member becomes a read of the anchor
//! and how the scheduler recognises an edit that reaches an array.
//!
//! [`array_footprint`] is the one statement of what a cell contributes to that
//! index, shared by the whole-workbook rebuild here and by the journal drain in
//! `model/mod.rs`, so the two cannot index by different rules. The formula
//! count lives here because the same walk maintains it.

use std::collections::HashMap;

use crate::dependency_graph::Position;
use crate::model::Model;
use crate::types::{ArrayKind, Cell};

/// The positions `cell` contributes to the array index, each with the anchor
/// that produces it: every position of a CSE anchor's declared rectangle (a
/// structural delete can drop a member cell while the anchor still owns and
/// refills the rectangle), a spill cell plus its anchor, and a dynamic anchor
/// unless its last result was a plain 1x1 scalar. A scalar dynamic anchor
/// (`=LET(..)`, a called LAMBDA, `=INDEX(..)`) stays out so it does not force
/// a Full pass; a blocked anchor (stored
/// `#SPILL!`) stays in, because full-mode same-pass readers observe the live
/// array's top-left value rather than the stored error, which incremental can
/// only match through the full pass; an unevaluated anchor's extent is unknown,
/// so it stays in too. Shared by the full-pass rebuild and the journal drain so
/// both index by the same rules.
pub(super) fn array_footprint(
    cell: &Cell,
    sheet: u32,
    row: i32,
    col: i32,
    out: &mut dyn FnMut(Position, Position),
) {
    let anchor = (sheet, row, col);
    match cell {
        Cell::ArrayFormula {
            kind: ArrayKind::Cse,
            r: (width, height),
            ..
        } => {
            for r in row..row + height {
                for c in col..col + width {
                    out((sheet, r, c), anchor);
                }
            }
        }
        Cell::ArrayFormula {
            kind: ArrayKind::Dynamic,
            v,
            ..
        } => {
            let scalar_result = match v {
                crate::types::FormulaValue::Error { ei, .. } => {
                    *ei != crate::expressions::token::Error::SPILL
                }
                crate::types::FormulaValue::Unevaluated => false,
                _ => true,
            };
            if !scalar_result {
                out(anchor, anchor);
            }
        }
        Cell::SpillCell { a, .. } => {
            let owner = (sheet, a.0, a.1);
            out(anchor, owner);
            out(owner, owner);
        }
        _ => {}
    }
}

impl Model<'_> {
    /// Records the positions of array and spill cells after a full pass, so the
    /// incremental path can fall back to full for any edit that reaches one. Must
    /// run after spilling, when the spill output cells exist. Between full
    /// passes the index is maintained without this walk: the journal drain adds
    /// the footprint of user-written cells, structural edits shift positions,
    /// and evaluation writes that change a footprint set `wrote_array_cells`,
    /// which sends the pass to Full and back here.
    pub(crate) fn collect_array_cells(&mut self) {
        let mut array_cells = HashMap::new();
        let mut formula_cell_count = 0;
        for ((sheet, row, col), cell) in self.cells_in_order() {
            if cell.get_formula().is_some() {
                formula_cell_count += 1;
            }
            array_footprint(cell, sheet, row, col, &mut |p, anchor| {
                array_cells.insert(p, anchor);
            });
        }
        self.formula_cell_count = formula_cell_count;
        self.formula_count_stale = false;
        self.graph.replace_arrays(array_cells);
    }

    /// Recounts formula cells without touching the array index. Used after a
    /// structural edit, which can add or remove whole rows of formula cells
    /// without a cell write for the journal to account against.
    ///
    /// Deliberately not [`Model::cells_in_order`]: a count is order-free, and
    /// this runs on the incremental path, where sorting the whole workbook to
    /// reach an order nothing reads would be the expense the fanout check
    /// exists to avoid.
    pub(crate) fn recount_formula_cells(&mut self) {
        self.formula_cell_count = self
            .workbook
            .worksheets
            .iter()
            .flat_map(|worksheet| worksheet.sheet_data.values())
            .flat_map(|row_data| row_data.values())
            .filter(|cell| cell.get_formula().is_some())
            .count();
        self.formula_count_stale = false;
    }

    /// Whether `position` holds an array formula that has never been evaluated.
    /// It is in the array index precisely because its extent is unknown, and it
    /// has no pre-pass value to compare against: every first evaluation would
    /// look like a mid-pass move.
    pub(super) fn is_unevaluated_array(&self, position: Position) -> bool {
        matches!(
            self.cell_at(position),
            Some(Cell::ArrayFormula {
                v: crate::types::FormulaValue::Unevaluated,
                ..
            })
        )
    }

    /// Parse-time dynamic-array anchors (`ArrayKind::Dynamic`) need the Full
    /// two-phase spill order even before they appear in `graph.arrays`.
    pub(super) fn is_dynamic_array_anchor(&self, position: Position) -> bool {
        matches!(
            self.cell_at(position),
            Some(Cell::ArrayFormula {
                kind: ArrayKind::Dynamic,
                ..
            })
        )
    }
}
