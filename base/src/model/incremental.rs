//! Incremental recalculation: observed-read graph, selective evaluation, and
//! the changed-cell delta it exposes.
//!
//! Edges are the reads recorded while a formula evaluates. The incremental
//! pass recomputes only the cells reachable from those that changed (plus
//! formulas that read RAND/NOW/TODAY) and records which ones moved, so
//! [`Model::take_changed_cells`] can report a precise delta.

use std::collections::{HashMap, HashSet};

use super::{ChangedCells, ChangedSinceRead};
use crate::cell::CellValue;
use crate::dependency_graph::Position;
#[cfg(feature = "recalc_verify")]
use crate::dependency_graph::RecalcMode;
use crate::expressions::types::CellReferenceIndex;
use crate::model::Model;
use crate::types::{Cell, CellType};

/// Below this many formula cells, incremental never falls back on fanout: the
/// bookkeeping is cheap in absolute terms and full has no edge to exploit.
const INCREMENTAL_FANOUT_FLOOR: usize = 1024;

/// Fall back to a full pass once an edit's fanout reaches this fraction of the
/// formula cells: `2` means half, past which full is cheaper than the
/// incremental bookkeeping it would save.
const INCREMENTAL_FANOUT_RATIO: usize = 2;

/// A cell's observable signature for incremental change detection: value, type
/// (so an error and a same-text literal differ), and dynamic link.
type ChangeKey = (CellType, ChangeValue, Option<crate::types::Link>);

/// Every cell's full observable state (`ChangeKey` plus conditional format),
/// used by the `Verify` check to compare incremental against full.
#[cfg(feature = "recalc_verify")]
type RenderSnapshot = HashMap<Position, (Option<ChangeKey>, Vec<crate::cf_types::CfCellResult>)>;

/// A cell value flattened for change detection. Numbers are kept as bits so a
/// `+0.0`/`-0.0` flip is seen and a `NaN` does not report as changed forever.
#[derive(PartialEq, Debug)]
enum ChangeValue {
    None,
    Boolean(bool),
    Number(u64),
    String(String),
}

/// Whether an evaluate stayed incremental or fell back to a full pass.
pub(crate) enum EvalPass {
    Incremental,
    Full,
}

impl Model<'_> {
    /// Records the positions of array and spill cells after a full pass, so the
    /// incremental path can fall back to full for any edit that reaches one. Must
    /// run after spilling, when the spill output cells exist.
    pub(crate) fn collect_array_cells(&mut self) {
        let mut array_cells = HashSet::new();
        let mut formula_cell_count = 0;
        for (sheet_index, worksheet) in self.workbook.worksheets.iter().enumerate() {
            let sheet = sheet_index as u32;
            for (row, row_data) in &worksheet.sheet_data {
                for (col, cell) in row_data {
                    if cell.get_formula().is_some() {
                        formula_cell_count += 1;
                    }
                    if matches!(cell, Cell::ArrayFormula { .. } | Cell::SpillCell { .. }) {
                        array_cells.insert((sheet, *row, *col));
                    }
                }
            }
        }
        self.formula_cell_count = formula_cell_count;
        self.graph.replace_arrays(array_cells);
    }

    /// When the pass stayed Incremental, runs a full pass and asserts they agree,
    /// and that the recorded delta names every cell whose observable state moved.
    /// A Full fallback has nothing to compare. Backs
    /// [`RecalcMode::Verify`](crate::dependency_graph::RecalcMode::Verify).
    #[cfg(feature = "recalc_verify")]
    pub(crate) fn verify_incremental_matches_full(&mut self) {
        let before = self.render_snapshot();
        // Completeness must run even if no consumer called take_changed_cells;
        // a fresh delta so a miss on this pass cannot hide behind an earlier one.
        let consumer =
            std::mem::replace(&mut self.changed_cells, ChangedCells::Delta(HashSet::new()));
        // Capture before evaluate: after_pass clears dirty.
        let seeds = self.graph.always_report_seeds();
        let pass = self.evaluate_selective();
        let this_pass =
            std::mem::replace(&mut self.changed_cells, ChangedCells::Delta(HashSet::new()));
        if matches!(pass, EvalPass::Incremental) {
            let incremental = self.render_snapshot();
            // RAND/NOW/TODAY re-roll. OFFSET stays in the compare when Incremental.
            // A top-level INDIRECT is a 1×1 dynamic array (Full, skipped).
            // SUM/PRODUCT(INDIRECT) stay Incremental and are compared.
            let tainted = self
                .graph
                .reachable(self.graph.always_dirty_cells().into_iter().collect());
            if let ChangedCells::Delta(delta) = &this_pass {
                for position in before.keys().chain(incremental.keys()) {
                    let changed = before.get(position) != incremental.get(position);
                    assert!(
                        tainted.contains(position) || !changed || delta.contains(position),
                        "cell {position:?} changed but is missing from the delta"
                    );
                }
                // Soundness: delta ⊆ (moved ∪ user/RAND seeds ∪ RAND cone).
                // `_set` writes the new value before evaluate, so a seed's
                // snapshot may not move even though the API reports it.
                // OFFSET is not a seed; it must not appear unless it moved.
                for position in delta {
                    let changed = before.get(position) != incremental.get(position);
                    assert!(
                        tainted.contains(position) || changed || seeds.contains(position),
                        "cell {position:?} is in the delta but did not change"
                    );
                }
            }
            self.changed_cells = merge_changed_cells(consumer, this_pass);
            // Shadow Full: run it on this model, then restore Incremental
            // state so the full pass cannot heal the graph we just used.
            let saved_workbook = self.workbook.clone();
            let saved_graph = self.graph.clone();
            let saved_cells = self.cells.clone();
            let saved_links = self.links.clone();
            let saved_cf = self.cf_cache.clone();
            let saved_support = self.support.clone();
            self.evaluate_full();
            let full = self.render_snapshot();
            let strip = |mut snapshot: RenderSnapshot| {
                snapshot.retain(|position, _| !tainted.contains(position));
                snapshot
            };
            assert_eq!(
                strip(incremental),
                strip(full),
                "incremental recalc diverged from full recompute"
            );
            self.workbook = saved_workbook;
            self.graph = saved_graph;
            self.cells = saved_cells;
            self.links = saved_links;
            self.cf_cache = saved_cf;
            self.support = saved_support;
        } else {
            // A full fallback has nothing to compare; a second full re-rolls RAND/NOW.
            // Still restore the consumer delta so a redundant evaluate is not a miss.
            self.changed_cells = merge_changed_cells(consumer, this_pass);
        }
    }

    /// Every cell's full observable state (value/type/link + conditional format),
    /// for the delta-completeness check: a cell whose state moves must be in the
    /// delta.
    #[cfg(feature = "recalc_verify")]
    fn render_snapshot(&self) -> RenderSnapshot {
        let mut positions: HashSet<Position> = self.cf_cache.keys().copied().collect();
        for c in self.get_all_cells() {
            positions.insert((c.index, c.row, c.column));
        }
        positions.extend(self.links.keys().copied());
        positions
            .into_iter()
            .map(|p| {
                (
                    p,
                    (
                        self.change_key(p),
                        self.cf_cache.get(&p).cloned().unwrap_or_default(),
                    ),
                )
            })
            .collect()
    }

    /// Adds to the delta any cell whose conditional-format result moved between
    /// `cf_before` and the rebuilt `cf_cache`. CF has no dependency edges, so a
    /// value or CF-rule change can move a cell's format with no value change.
    fn record_cf_changes(
        &mut self,
        cf_before: HashMap<Position, Vec<crate::cf_types::CfCellResult>>,
    ) {
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

    pub(crate) fn evaluate_selective(&mut self) -> EvalPass {
        if self.graph.should_recompute_full() {
            // A full from a shape-changing edit or the first pass may change any
            // cell, so drop the delta. A trailing delete can leave dirty empty
            // (nothing below to shift) while still emptying cells; catch that
            // before the CF-diff branch, which would run evaluate_full, clear
            // the flag in after_pass, and report Cells([]).
            // A redundant full with nothing pending keeps the delta, unless
            // RAND/NOW/TODAY are present: a full pass re-rolls those.
            // OFFSET does not re-roll and must not wipe the delta.
            if self.graph.take_structural_unknown()
                || self.graph.full_reflects_change()
                || !self.graph.always_dirty_cells().is_empty()
            {
                self.evaluate_full_untracked();
            } else {
                // A redundant full preserves the delta unless values actually
                // moved (e.g. a spill that takes two passes). Diff cells and CF.
                let before: HashMap<Position, Option<ChangeKey>> = self
                    .get_all_cells()
                    .into_iter()
                    .map(|c| {
                        let p = (c.index, c.row, c.column);
                        (p, self.change_key(p))
                    })
                    .collect();
                let cf_before = self.cf_cache.clone();
                self.evaluate_full_and_follow_up_new_arrays();
                let after: Vec<(Position, Option<ChangeKey>)> = self
                    .get_all_cells()
                    .into_iter()
                    .map(|c| {
                        let p = (c.index, c.row, c.column);
                        (p, self.change_key(p))
                    })
                    .collect();
                if let ChangedCells::Delta(delta) = &mut self.changed_cells {
                    let mut seen = HashSet::new();
                    for (p, now) in after {
                        seen.insert(p);
                        if before.get(&p) != Some(&now) {
                            delta.insert(p);
                        }
                    }
                    for p in before.keys() {
                        if !seen.contains(p) {
                            delta.insert(*p);
                        }
                    }
                }
                self.record_cf_changes(cf_before);
            }
            return EvalPass::Full;
        }
        // Formulas that read a non-sheet input re-roll every pass and are always
        // reported. OFFSET/INDIRECT are not always-dirty: their actual targets
        // are traced edges, so they re-run only when a precedent moves.
        let always_dirty: Vec<Position> = self.graph.always_dirty_cells().into_iter().collect();
        let mut always_report: Vec<Position> = self.graph.peek_dirty();
        always_report.extend(always_dirty.iter().copied());
        for &cell in &always_dirty {
            self.graph.mark_dirty(cell);
        }
        let (seeds, affected) = self.graph.take_seeds_and_affected();
        // A wide-fanout edit reaches most of the workbook, where incremental
        // bookkeeping costs about as much as it saves; past half the formulas a
        // full pass is cheaper. The floor keeps small workbooks on the fast path.
        // Verify skips this: it is a performance fallback, not a correctness one.
        if self.should_fallback_fanout(affected.len()) {
            self.evaluate_full_untracked();
            return EvalPass::Full;
        }
        // Array and spill cells need the full pass's two-phase ordering.
        if affected.iter().any(|cell| self.graph.arrays.contains(cell)) {
            self.evaluate_full_untracked();
            return EvalPass::Full;
        }
        // Recompute the affected cells and collect the ones whose value actually
        // moved. A cycle in the affected set has no topological order, so fall
        // back to recomputing the whole set, where `evaluate_cell`'s recursion
        // still reports `#CIRC!`.
        let changed = match self.graph.topo_order(&affected) {
            Some(order) => {
                self.recompute_frontier(affected, &seeds, &always_report, order, HashSet::new())
            }
            None => self.recompute_all(affected, &always_report),
        };
        // Record only the changed cells for `take_changed_cells`, unless a full
        // pass has already marked everything changed since the last read, or an
        // insert/delete moved cells the dirty cone does not name.
        if self.graph.take_structural_unknown() {
            self.changed_cells = ChangedCells::All;
        } else if let ChangedCells::Delta(delta) = &mut self.changed_cells {
            delta.extend(changed);
        }
        self.graph.after_pass();
        let cf_before = self.cf_cache.clone();
        self.evaluate_conditional_formatting();
        self.record_cf_changes(cf_before);
        EvalPass::Incremental
    }

    /// A cell's observable signature: value, type (so an error and a same-text
    /// literal differ), and dynamic link (a HYPERLINK target can move under a
    /// fixed label).
    fn change_key(&self, position @ (sheet, row, column): Position) -> Option<ChangeKey> {
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

    /// Clears a cell's cached state so the next `evaluate_cell` recomputes it.
    /// Drops the dynamic link too, as a full pass would, so a cell that no longer
    /// resolves to a `HYPERLINK` does not keep a stale one.
    fn invalidate(&mut self, position: Position) {
        self.cells.remove(&position);
        self.links.remove(&position);
    }

    /// Recomputes `must_run` in topological order. `always_report` (user edits,
    /// RAND) always counts as changed and propagates. An unchanged non-report
    /// cell stops the fanout there.
    ///
    /// Memo is dropped on the whole cone first so a newly recorded read
    /// (OFFSET's actual target) cannot see a stale `Evaluated` helper that is
    /// also in the cone. Helpers pulled in via `evaluate_cell` are restored if
    /// the frontier then skips them, so a later CF formula does not re-run them
    /// unscoped.
    fn recompute_frontier(
        &mut self,
        affected: HashSet<Position>,
        must_run: &[Position],
        always_report: &[Position],
        order: Vec<Position>,
        extra_scope: HashSet<Position>,
    ) -> Vec<Position> {
        let before: HashMap<Position, Option<ChangeKey>> =
            affected.iter().map(|&p| (p, self.change_key(p))).collect();
        let mut scope = affected.clone();
        scope.extend(extra_scope);
        self.recompute_scope = Some(scope);
        let report: HashSet<Position> = always_report.iter().copied().collect();
        let mut stale: HashSet<Position> = must_run.iter().copied().collect();
        let mut changed = HashSet::new();
        let saved: HashMap<Position, _> = affected
            .iter()
            .filter_map(|&p| self.cells.get(&p).cloned().map(|state| (p, state)))
            .collect();
        for &position in &affected {
            self.cells.remove(&position);
        }
        for position in order {
            if !stale.contains(&position) {
                continue;
            }
            self.invalidate(position);
            let (sheet, row, column) = position;
            self.evaluate_cell(CellReferenceIndex { sheet, row, column });
            if report.contains(&position) || self.change_key(position) != before[&position] {
                changed.insert(position);
                stale.extend(self.graph.dependents_of(position));
            }
        }
        for &position in &affected {
            if let std::collections::hash_map::Entry::Vacant(entry) = self.cells.entry(position) {
                if let Some(state) = saved.get(&position) {
                    entry.insert(state.clone());
                }
            }
        }
        self.recompute_scope = None;
        // OFFSET/INDIRECT can recompute a helper via evaluate_cell before this
        // loop reaches it; that cell never entered `stale`.
        for &position in &affected {
            if report.contains(&position) || self.change_key(position) != before[&position] {
                changed.insert(position);
            }
        }
        changed.into_iter().collect()
    }

    /// Recomputes the whole affected set, used when a cycle prevents ordering.
    /// Returns `always_report` plus every other cell whose value moved.
    fn recompute_all(
        &mut self,
        affected: HashSet<Position>,
        always_report: &[Position],
    ) -> Vec<Position> {
        let mut order: Vec<Position> = affected.iter().copied().collect();
        order.sort_unstable();
        let before: HashMap<Position, Option<ChangeKey>> =
            order.iter().map(|&p| (p, self.change_key(p))).collect();
        for &position in &order {
            self.invalidate(position);
        }
        self.recompute_scope = Some(affected);
        for &(sheet, row, column) in &order {
            self.evaluate_cell(CellReferenceIndex { sheet, row, column });
        }
        self.recompute_scope = None;
        let report: HashSet<Position> = always_report.iter().copied().collect();
        order
            .into_iter()
            .filter(|p| report.contains(p) || self.change_key(*p) != before[p])
            .collect()
    }

    /// Full recompute whose result is not expressible as a delta: it may have
    /// changed any cell, so the next `take_changed_cells` reports `Everything`.
    pub(crate) fn evaluate_full_untracked(&mut self) {
        self.evaluate_full_and_follow_up_new_arrays();
        self.changed_cells = ChangedCells::All;
    }

    /// A newly observed dynamic-array anchor may not have seen a SEQUENCE
    /// that spilled later in the same pass (`E15#` after `=SEQUENCE(3)`).
    /// Leave it dirty so the next evaluate takes the arrays→Full path and
    /// matches a second Full-mode pass.
    fn evaluate_full_and_follow_up_new_arrays(&mut self) {
        let before = self.graph.arrays.snapshot();
        self.evaluate_full();
        let new: Vec<Position> = self
            .graph
            .arrays
            .snapshot()
            .into_iter()
            .filter(|p| !before.contains(p))
            .collect();
        for p in new {
            self.graph.mark_dirty(p);
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

    /// Performance-only: a wide cone is cheaper as a full pass. Verify stays on
    /// the incremental path so the oracle still compares the two.
    fn should_fallback_fanout(&self, fanout: usize) -> bool {
        #[cfg(feature = "recalc_verify")]
        if self.recalc_mode == RecalcMode::Verify {
            return false;
        }
        self.formula_cell_count >= INCREMENTAL_FANOUT_FLOOR
            && fanout * INCREMENTAL_FANOUT_RATIO >= self.formula_cell_count
    }
}

/// Unions two change records. `All` wins; otherwise the cells are merged.
#[cfg(feature = "recalc_verify")]
fn merge_changed_cells(consumer: ChangedCells, this_pass: ChangedCells) -> ChangedCells {
    match (consumer, this_pass) {
        (ChangedCells::All, _) | (_, ChangedCells::All) => ChangedCells::All,
        (ChangedCells::Delta(mut a), ChangedCells::Delta(b)) => {
            a.extend(b);
            ChangedCells::Delta(a)
        }
    }
}
