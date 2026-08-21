//! Range composition for associative aggregates (SUM, MIN, MAX, COUNT).
//!
//! `SUM(A1:A100)` over a column of running totals recomputes the whole range for
//! every row, so `SUM(A1:A1)…SUM(A1:A100)` costs O(n²). Mirroring HyperFormula's
//! range criss-cross, a range is reduced by reusing the result of the range one
//! row shorter plus its last row, which brings the family down to O(n). The
//! shorter range is reused only when it is itself referenced by a formula, so a
//! lone range keeps its direct scan and pays no speculative overhead.
//!
//! Results are memoized per evaluation pass (see [`Model::evaluate`]); the cache
//! is cleared whenever cell values are recomputed, so it never outlives the
//! values it summarizes.

use std::collections::{HashMap, HashSet};

use super::{CellOrRange, Model};
use crate::calc_result::CalcResult;
use crate::dependency_graph::{Area, RecalcMode};
use crate::expressions::token::Error;
use crate::expressions::types::CellReferenceIndex;

/// Per-pass memo of range reductions, keyed by range and reducer.
pub(crate) type RangeReduceCache = HashMap<(Area, RangeReducer), RangeAgg>;

/// An associative reduction over the numeric cells of a range.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RangeReducer {
    Sum,
    Min,
    Max,
    /// Counts numeric cells. Unlike the others it ignores errors rather than
    /// propagating them, matching COUNT.
    Count,
}

/// A running reduction: a number, or the first error seen in row-major order,
/// which error-propagating functions surface just as a direct scan would.
#[derive(Clone)]
pub(crate) enum RangeAgg {
    Number(f64),
    Error(CalcResult),
}

impl RangeAgg {
    /// Mid-cycle `#CIRC` is not a real aggregate. Caching it makes a later
    /// `SUM` of the same range depend on evaluation order.
    fn cacheable(&self) -> bool {
        !matches!(
            self,
            RangeAgg::Error(CalcResult::Error {
                error: Error::CIRC,
                ..
            })
        )
    }
}

impl RangeReducer {
    /// The value of an empty range: 0 for Sum/Count, NaN for Min/Max so the first
    /// number wins (`x.min(NaN) == x`) and an all-empty range stays NaN, which the
    /// caller maps to 0 just as its direct scan does.
    fn identity_value(self) -> f64 {
        match self {
            RangeReducer::Sum | RangeReducer::Count => 0.0,
            RangeReducer::Min | RangeReducer::Max => f64::NAN,
        }
    }

    /// A single numeric cell's contribution: its value, or 1 when counting.
    fn contribution(self, value: f64) -> f64 {
        match self {
            RangeReducer::Count => 1.0,
            _ => value,
        }
    }

    /// Merges two numeric sub-results.
    fn merge(self, a: f64, b: f64) -> f64 {
        match self {
            RangeReducer::Sum | RangeReducer::Count => a + b,
            // `b.min(a)`, not `a.min(b)`: main scans `value.min(result)`, and
            // minNum's choice between +0 and -0 is operand-order dependent on x86.
            RangeReducer::Min => b.min(a),
            RangeReducer::Max => b.max(a),
        }
    }

    /// Whether an error cell aborts the reduction (Sum/Min/Max) or is ignored
    /// (Count).
    fn propagates_errors(self) -> bool {
        !matches!(self, RangeReducer::Count)
    }

    fn identity(self) -> RangeAgg {
        RangeAgg::Number(self.identity_value())
    }

    /// Combines two sub-results, keeping the first error in row-major order.
    fn combine(self, left: RangeAgg, right: RangeAgg) -> RangeAgg {
        match (left, right) {
            (RangeAgg::Error(error), _) | (_, RangeAgg::Error(error)) => RangeAgg::Error(error),
            (RangeAgg::Number(a), RangeAgg::Number(b)) => RangeAgg::Number(self.merge(a, b)),
        }
    }
}

impl Model<'_> {
    /// Records the ranges referenced by formulas, per sheet, so range composition
    /// knows when reusing a one-row-shorter range is worthwhile. Incremental and
    /// Verify only; default Full skips the walk. Stored on the graph and shifted
    /// with it, so an incremental insert or delete keeps the set without a rebuild.
    pub(crate) fn collect_referenced_ranges(&mut self) {
        let mut referenced: HashMap<u32, HashSet<Area>> = HashMap::new();
        for (sheet_index, worksheet) in self.workbook.worksheets.iter().enumerate() {
            let sheet = sheet_index as u32;
            for (row, row_data) in &worksheet.sheet_data {
                for (col, cell) in row_data {
                    if let Some(formula) = cell.get_formula() {
                        let context = CellReferenceIndex {
                            sheet,
                            row: *row,
                            column: *col,
                        };
                        let node = &self.parsed_formulas[sheet as usize][formula as usize].0;
                        let mut refs = Vec::new();
                        self.collect_references(node, context, &mut refs, &mut HashSet::new());
                        for reference in refs {
                            if let CellOrRange::Range(area) = reference {
                                referenced.entry(area.0).or_default().insert(area);
                            }
                        }
                    }
                }
            }
        }
        self.graph.replace_referenced(referenced);
    }

    /// Reduces `reducer` over `sheet!row1:row2 x column1:column2`, memoized per
    /// pass. Bounds must already be clamped to the sheet (the caller resolves
    /// full-column/row references first, as a direct scan would). Cells that are
    /// not numbers are ignored and the first error is propagated, matching the
    /// aggregating functions' own semantics.
    pub(crate) fn reduce_range(
        &mut self,
        sheet: u32,
        row1: i32,
        column1: i32,
        row2: i32,
        column2: i32,
        reducer: RangeReducer,
    ) -> RangeAgg {
        // Default Full must match pre-composition fn_sum: one accumulator,
        // left-to-right then down. Per-row subtotals re-associate floats.
        if self.recalc_mode == RecalcMode::Full {
            return self.reduce_range_direct(
                sheet,
                row1,
                column1,
                row2,
                column2,
                reducer,
                reducer.identity(),
            );
        }
        let key = ((sheet, row1, column1, row2, column2), reducer);
        if let Some(agg) = self.cached_agg(&key) {
            return agg;
        }
        // Descend to the deepest reusable sub-range: a cached prefix, the first
        // row, or the point where the one-shorter range stops being referenced
        // (so a lone range is scanned directly rather than building a chain).
        let mut base = row2;
        while base > row1 {
            let base_key = ((sheet, row1, column1, base, column2), reducer);
            if self.cached_agg(&base_key).is_some() {
                break;
            }
            let shorter = (sheet, row1, column1, base - 1, column2);
            if !self.graph.referenced.contains(&shorter) {
                break;
            }
            base -= 1;
        }
        // Reuse the cached prefix for `row1..=base`, or scan that block directly.
        let base_key = ((sheet, row1, column1, base, column2), reducer);
        let mut circ_seen = false;
        let mut acc = if let Some(agg) = self.cached_agg(&base_key) {
            agg
        } else {
            let mut acc = reducer.identity();
            for row in row1..=base {
                let (row_agg, row_circ) = self.reduce_row(sheet, row, column1, column2, reducer);
                circ_seen |= row_circ;
                acc = reducer.combine(acc, row_agg);
                if matches!(acc, RangeAgg::Error(_)) {
                    break;
                }
            }
            self.cache_agg(base_key, acc.clone(), circ_seen);
            acc
        };
        // Fold the remaining rows one at a time, caching every prefix so that
        // overlapping ranges reuse them. Once an error is seen it wins for every
        // longer prefix, so stop scanning and just record it.
        for row in (base + 1)..=row2 {
            let prefix_key = ((sheet, row1, column1, row, column2), reducer);
            if matches!(acc, RangeAgg::Error(_)) {
                self.cache_agg(prefix_key, acc.clone(), circ_seen);
                continue;
            }
            let (row_agg, row_circ) = self.reduce_row(sheet, row, column1, column2, reducer);
            circ_seen |= row_circ;
            acc = reducer.combine(acc, row_agg);
            self.cache_agg(prefix_key, acc.clone(), circ_seen);
        }
        acc
    }

    /// Streams the range into `acc` in Full, or combines `acc` with a composed
    /// subtotal in Incremental. Full callers pass the outer SUM/MIN/COUNT
    /// accumulator so `=SUM(5, A1:A3)` keeps pre-composition association.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fold_range(
        &mut self,
        sheet: u32,
        row1: i32,
        column1: i32,
        row2: i32,
        column2: i32,
        reducer: RangeReducer,
        acc: RangeAgg,
    ) -> RangeAgg {
        if self.recalc_mode == RecalcMode::Full {
            return self.reduce_range_direct(sheet, row1, column1, row2, column2, reducer, acc);
        }
        reducer.combine(
            acc,
            self.reduce_range(sheet, row1, column1, row2, column2, reducer),
        )
    }

    /// Pre-composition scan: one accumulator in row-major order. Used in Full
    /// so default-mode users keep main's floating-point association.
    #[allow(clippy::too_many_arguments)]
    fn reduce_range_direct(
        &mut self,
        sheet: u32,
        row1: i32,
        column1: i32,
        row2: i32,
        column2: i32,
        reducer: RangeReducer,
        mut acc: RangeAgg,
    ) -> RangeAgg {
        for row in row1..=row2 {
            for column in column1..=column2 {
                match self.evaluate_cell(CellReferenceIndex { sheet, row, column }) {
                    CalcResult::Number(value) => {
                        acc = reducer.combine(acc, RangeAgg::Number(reducer.contribution(value)));
                    }
                    error @ CalcResult::Error { .. } if reducer.propagates_errors() => {
                        return RangeAgg::Error(error);
                    }
                    _ => {}
                }
            }
        }
        acc
    }

    fn cached_agg(&self, key: &(Area, RangeReducer)) -> Option<RangeAgg> {
        self.range_reduce_cache
            .get(key)
            .filter(|agg| agg.cacheable())
            .cloned()
    }

    fn cache_agg(&mut self, key: (Area, RangeReducer), agg: RangeAgg, circ_seen: bool) {
        // COUNT ignores errors, so a mid-cycle `#CIRC` becomes a Number.
        // Caching that poisons later scans of the same range.
        if !circ_seen && agg.cacheable() {
            self.range_reduce_cache.insert(key, agg);
        }
    }

    /// Reduces a single row of a range across `column1..=column2`, evaluating each
    /// cell so its value and the range's dependency edge are recorded just as a
    /// direct scan would. Returns on the first error, left to right.
    ///
    /// The row is reduced into its own accumulator before being combined with the
    /// other rows, so a multi-row, multi-column SUM adds in a different order than
    /// a single left-to-right scan. This re-association (which HyperFormula also
    /// does) can shift the result by a floating-point rounding step; the aggregate
    /// is preserved, not the exact summation order. Min/Max/Count are associative,
    /// so their result is independent of order.
    fn reduce_row(
        &mut self,
        sheet: u32,
        row: i32,
        column1: i32,
        column2: i32,
        reducer: RangeReducer,
    ) -> (RangeAgg, bool) {
        let mut acc = reducer.identity_value();
        let mut circ_seen = false;
        for column in column1..=column2 {
            match self.evaluate_cell(CellReferenceIndex { sheet, row, column }) {
                CalcResult::Number(value) => acc = reducer.merge(acc, reducer.contribution(value)),
                error @ CalcResult::Error {
                    error: Error::CIRC, ..
                } => {
                    circ_seen = true;
                    if reducer.propagates_errors() {
                        return (RangeAgg::Error(error), true);
                    }
                }
                error @ CalcResult::Error { .. } if reducer.propagates_errors() => {
                    return (RangeAgg::Error(error), circ_seen);
                }
                _ => {}
            }
        }
        (RangeAgg::Number(acc), circ_seen)
    }
}
