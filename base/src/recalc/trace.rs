use crate::dependency_graph::{Area, Position};

/// A non-cell input a formula read during evaluation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Input {
    RowHidden(u32, i32),
    /// No function records this yet: `SUBTOTAL(1xx)` excludes hidden rows
    /// only, matching Excel, so column visibility is not an input to anything
    /// implemented. The variant, its journal write, and its drain consumer are
    /// wired end-to-end for a future reader (e.g. `CELL("width")`).
    #[allow(dead_code)]
    ColHidden(u32, i32),
    OwnCoord(Position),
    FormulaText(Position),
    Name {
        name: String,
        scope: Option<u32>,
    },
    Clock,
    Random,
    SheetStructure,
    /// CELL/INFO observe workbook environment, not cell values.
    Environment,
    /// A read whose target was computed (OFFSET/INDIRECT). Structural edits
    /// re-dirty these formulas so they re-resolve instead of shifting a snapshot.
    Computed,
}

/// Cells, rectangles, and non-cell inputs observed while evaluating one formula.
/// A covering rect suppresses per-cell edges for the same read (SUM(A:A) stays
/// one range vertex, not a million cell edges).
#[derive(Clone, Debug, Default)]
pub(crate) struct ReadSet {
    /// Single cells read, none of them covered by a rect in `rects`.
    pub cells: Vec<Position>,
    /// Rectangles read whole. Deduplicated, and never expanded to cells.
    pub rects: Vec<Area>,
    /// Non-cell inputs read. Deduplicated.
    pub inputs: Vec<Input>,
}

impl ReadSet {
    /// Records a read of one cell, unless a rect already recorded covers it.
    /// Idempotent.
    pub(crate) fn record_cell(&mut self, cell: Position) {
        if self.rects.iter().any(|area| area_contains(*area, cell)) {
            return;
        }
        if !self.cells.contains(&cell) {
            self.cells.push(cell);
        }
    }

    /// Records a read of a whole rectangle, dropping the per-cell edges it
    /// now covers so the same read is one range vertex, not many. Idempotent.
    pub(crate) fn record_rect(&mut self, area: Area) {
        if !self.rects.contains(&area) {
            self.rects.push(area);
            self.cells.retain(|cell| !area_contains(area, *cell));
        }
    }

    /// Records a read of a non-cell input. Idempotent.
    pub(crate) fn record_input(&mut self, input: Input) {
        if !self.inputs.contains(&input) {
            self.inputs.push(input);
        }
    }
}

fn area_contains(area: Area, cell: Position) -> bool {
    let (sheet, row, column) = cell;
    let (a_sheet, r1, c1, r2, c2) = area;
    sheet == a_sheet && row >= r1 && row <= r2 && column >= c1 && column <= c2
}
