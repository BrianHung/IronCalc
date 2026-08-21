use crate::dependency_graph::{Area, Position};

/// A non-cell input a formula read during evaluation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Input {
    RowHidden(u32, i32),
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
    #[allow(dead_code)]
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
    pub cells: Vec<Position>,
    pub rects: Vec<Area>,
    pub inputs: Vec<Input>,
}

impl ReadSet {
    pub(crate) fn record_cell(&mut self, cell: Position) {
        if self.rects.iter().any(|area| area_contains(*area, cell)) {
            return;
        }
        if !self.cells.contains(&cell) {
            self.cells.push(cell);
        }
    }

    pub(crate) fn record_rect(&mut self, area: Area) {
        if !self.rects.contains(&area) {
            self.rects.push(area);
            self.cells.retain(|cell| !area_contains(area, *cell));
        }
    }

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
