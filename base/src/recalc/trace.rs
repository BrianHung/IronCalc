use crate::dependency_graph::{Area, Position};

/// A non-cell input a formula read during evaluation.
///
/// The line- and rectangle-keyed variants carry an *extent*, not a single
/// coordinate: a read of one row is the degenerate span `(r, r)` and a read of
/// one cell the degenerate rect `(r, c, r, c)`. The extent is what lets the
/// tracer apply the same economy to inputs that it applies to cells --- see
/// [`ReadSet::record_input`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Input {
    /// Visibility of rows `row1..=row2` of a sheet.
    RowHidden(u32, i32, i32),
    /// Visibility of columns `column1..=column2` of a sheet.
    ///
    /// No function records this yet: `SUBTOTAL(1xx)` excludes hidden rows
    /// only, matching Excel, so column visibility is not an input to anything
    /// implemented. The variant, its journal write, and its drain consumer are
    /// wired end-to-end for a future reader (e.g. `CELL("width")`).
    #[allow(dead_code)]
    ColHidden(u32, i32, i32),
    OwnCoord(Position),
    /// Formula-ness (or formula text) of every cell in a rectangle.
    FormulaText(Area),
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

impl Input {
    /// Whether every read `other` stands for is already stood for by `self`.
    ///
    /// Only the extent-carrying variants have anything to compare; for the rest
    /// this is plain equality, which is all [`ReadSet::record_input`] needs to
    /// deduplicate them.
    fn covers(&self, other: &Input) -> bool {
        match (self, other) {
            (Input::RowHidden(s, a1, a2), Input::RowHidden(t, b1, b2))
            | (Input::ColHidden(s, a1, a2), Input::ColHidden(t, b1, b2)) => {
                s == t && a1 <= b1 && b2 <= a2
            }
            (Input::FormulaText(a), Input::FormulaText(b)) => area_contains_area(*a, *b),
            _ => self == other,
        }
    }
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
    /// Non-cell inputs read. Deduplicated, and each widened to the rect it was
    /// read beneath.
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
        if self.rects.contains(&area) {
            return;
        }
        self.rects.push(area);
        self.cells.retain(|cell| !area_contains(area, *cell));
        // Inputs recorded before this rect landed widen to it now, the same way
        // cells recorded before it are dropped now. Re-recording them merges the
        // ones that widen to the same extent.
        for input in std::mem::take(&mut self.inputs) {
            self.record_input(input);
        }
    }

    /// Records a read of a non-cell input, widened to the rect it was read
    /// beneath. Idempotent.
    ///
    /// The economy is the one `record_rect` applies to cells. A walk over a
    /// range asks the same question of every line or cell it passes ---
    /// `SUBTOTAL(103,A:A)` asks whether each row is hidden, and whether each
    /// cell holds a nested `SUBTOTAL` --- and that is one input over the rect,
    /// not one per row. Widening over-approximates (the formula re-runs for a
    /// row inside the rect it might not have reached, because the walk stopped
    /// at the used range) and never under-approximates, which is the same
    /// trade the recorded rect itself makes.
    ///
    /// Without it a whole-column `SUBTOTAL` records an input per row: the
    /// linear dedup below turns quadratic, and the graph grows an input edge
    /// per row of the sheet's used range.
    pub(crate) fn record_input(&mut self, input: Input) {
        let input = self.widen(input);
        if self.inputs.iter().any(|held| held.covers(&input)) {
            return;
        }
        self.inputs.retain(|held| !input.covers(held));
        self.inputs.push(input);
    }

    /// `input` grown to the extent of a recorded rect containing it, if there
    /// is one. Variants that carry no extent are returned unchanged.
    fn widen(&self, input: Input) -> Input {
        match input {
            Input::RowHidden(sheet, row1, row2) => {
                match self
                    .rects
                    .iter()
                    .find(|(s, r1, _, r2, _)| *s == sheet && *r1 <= row1 && row2 <= *r2)
                {
                    Some((_, r1, _, r2, _)) => Input::RowHidden(sheet, *r1, *r2),
                    None => Input::RowHidden(sheet, row1, row2),
                }
            }
            Input::ColHidden(sheet, column1, column2) => {
                match self
                    .rects
                    .iter()
                    .find(|(s, _, c1, _, c2)| *s == sheet && *c1 <= column1 && column2 <= *c2)
                {
                    Some((_, _, c1, _, c2)) => Input::ColHidden(sheet, *c1, *c2),
                    None => Input::ColHidden(sheet, column1, column2),
                }
            }
            Input::FormulaText(area) => {
                match self
                    .rects
                    .iter()
                    .find(|rect| area_contains_area(**rect, area))
                {
                    Some(rect) => Input::FormulaText(*rect),
                    None => Input::FormulaText(area),
                }
            }
            other => other,
        }
    }
}

pub(crate) fn area_contains(area: Area, cell: Position) -> bool {
    let (sheet, row, column) = cell;
    let (a_sheet, r1, c1, r2, c2) = area;
    sheet == a_sheet && row >= r1 && row <= r2 && column >= c1 && column <= c2
}

/// Whether `outer` contains all of `inner`.
fn area_contains_area(outer: Area, inner: Area) -> bool {
    let (o_sheet, o_r1, o_c1, o_r2, o_c2) = outer;
    let (i_sheet, i_r1, i_c1, i_r2, i_c2) = inner;
    o_sheet == i_sheet && o_r1 <= i_r1 && o_c1 <= i_c1 && i_r2 <= o_r2 && i_c2 <= o_c2
}
