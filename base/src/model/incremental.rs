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
use crate::types::{ArrayKind, Cell, CellType};

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

/// The positions `cell` contributes to the array index: every position of a
/// CSE anchor's declared rectangle (a structural delete can drop a member cell
/// while the anchor still owns and refills the rectangle), a spill cell plus
/// its anchor, and a dynamic anchor unless its last result was a plain 1x1
/// scalar. A scalar dynamic anchor (`=LET(..)`, a called LAMBDA, `=INDEX(..)`)
/// stays out so it does not force a Full pass; a blocked anchor (stored
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
    out: &mut dyn FnMut(Position),
) {
    match cell {
        Cell::ArrayFormula {
            kind: ArrayKind::Cse,
            r: (width, height),
            ..
        } => {
            for r in row..row + height {
                for c in col..col + width {
                    out((sheet, r, c));
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
                out((sheet, row, col));
            }
        }
        Cell::SpillCell { a, .. } => {
            out((sheet, row, col));
            out((sheet, a.0, a.1));
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
        let mut array_cells = HashSet::new();
        let mut formula_cell_count = 0;
        for (sheet_index, worksheet) in self.workbook.worksheets.iter().enumerate() {
            let sheet = sheet_index as u32;
            let mut sorted_rows: Vec<i32> = worksheet.sheet_data.keys().copied().collect();
            sorted_rows.sort_unstable();
            for row in sorted_rows {
                let row_data = &worksheet.sheet_data[&row];
                let mut sorted_cols: Vec<i32> = row_data.keys().copied().collect();
                sorted_cols.sort_unstable();
                for col in sorted_cols {
                    let cell = &row_data[&col];
                    if cell.get_formula().is_some() {
                        formula_cell_count += 1;
                    }
                    array_footprint(cell, sheet, row, col, &mut |p| {
                        array_cells.insert(p);
                    });
                }
            }
        }
        self.formula_cell_count = formula_cell_count;
        self.formula_count_stale = false;
        self.graph.replace_arrays(array_cells);
    }

    /// Recounts formula cells without touching the array index. Used after a
    /// structural edit, which can add or remove whole rows of formula cells
    /// without a cell write for the journal to account against.
    pub(crate) fn recount_formula_cells(&mut self) {
        let mut formula_cell_count = 0;
        for worksheet in &self.workbook.worksheets {
            for row_data in worksheet.sheet_data.values() {
                for cell in row_data.values() {
                    if cell.get_formula().is_some() {
                        formula_cell_count += 1;
                    }
                }
            }
        }
        self.formula_cell_count = formula_cell_count;
        self.formula_count_stale = false;
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
        // The pre-pass always-dirty set, which is the set `evaluate_selective`
        // reads at pass start to seed `always_report`. Liveness is asserted
        // against this one, not against the post-pass set: see below.
        let always_dirty_before = self.graph.always_dirty_cells();
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
                // Liveness. Both checks above consult the always-dirty set to
                // excuse a cell, and the value comparison below strips its
                // whole cone, so a pass that silently stopped re-running the
                // volatiles would read as clean everywhere else. Being
                // reported on every pass is what is left to assert.
                //
                // Against the PRE-pass set, because that is the set the pass
                // seeded `always_report` from. A cell whose branch flips INTO
                // `RAND()` on this pass records the input only as it evaluates:
                // it joins the always-dirty set mid-pass, was never a seed, and
                // if its value did not move the delta rightly leaves it out.
                // It is asserted from the next pass on. The reverse transition
                // is still asserted here -- a cell leaving volatility was in
                // the pre-pass set, so it seeded `always_report` and must be
                // reported on the pass that drops it -- and so is the steady
                // state, where the two sets are the same.
                for position in &always_dirty_before {
                    assert!(
                        delta.contains(position),
                        "always-dirty cell {position:?} was not reported"
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
            self.assert_stored_matches_live();
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

    /// Out-of-scope incremental reads return the stored value. Re-evaluating a
    /// non-volatile formula in a one-cell scratch frame must agree with that
    /// store (class C: `FormulaValue::Empty` vs a live blank).
    #[cfg(feature = "recalc_verify")]
    fn assert_stored_matches_live(&mut self) {
        let skip = self.graph.always_dirty_cells();
        let cells = self.get_all_cells();
        let saved_cells = self.cells.clone();
        let saved_graph = self.graph.clone();
        let saved_workbook = self.workbook.clone();
        let saved_scope = self.recompute_scope.clone();
        for c in cells {
            let position = (c.index, c.row, c.column);
            if skip.contains(&position) {
                continue;
            }
            if self
                .get_cell_formula(c.index, c.row, c.column)
                .ok()
                .flatten()
                .is_none()
            {
                continue;
            }
            // Spills rewrite a rectangle; a one-cell scratch frame is not a
            // faithful re-eval of an array formula.
            if matches!(
                self.workbook
                    .worksheet(c.index)
                    .ok()
                    .and_then(|ws| ws.cell(c.row, c.column)),
                Some(Cell::ArrayFormula { .. } | Cell::SpillCell { .. })
            ) {
                continue;
            }
            // A cycle has no fixpoint: a cell on one stores a value computed
            // from mid-cycle reads (COUNT swallows a mid-cycle #CIRC! into a
            // number), so stored == live cannot hold there. Skipping when the
            // cell's cone has no topological order over-skips cells that
            // merely feed a downstream cycle, which is an acceptable loss of
            // oracle coverage in exchange for no false alarms.
            let cone = self.graph.reachable(vec![position]);
            if self.graph.topo_order(&cone).is_none() {
                continue;
            }
            let before = self.change_key(position);
            self.recompute_scope = Some(HashSet::from([position]));
            self.cells.remove(&position);
            let _ = self.evaluate_cell(CellReferenceIndex {
                sheet: c.index,
                row: c.row,
                column: c.column,
            });
            let after = self.change_key(position);
            assert_eq!(
                before, after,
                "stored value diverged from a live re-eval at {position:?}"
            );
        }
        self.cells = saved_cells;
        self.graph = saved_graph;
        self.workbook = saved_workbook;
        self.recompute_scope = saved_scope;
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
        self.pass_generation = self.pass_generation.wrapping_add(1);
        let write_seeds = std::mem::take(&mut self.write_seeds);
        // Any leftover flag belongs to a pass that ended in a full rebuild of
        // the array index; only footprint writes from this pass's frontier
        // matter below.
        self.wrote_array_cells = false;
        // The previous pass was not a fixed point: a spill or CSE footprint
        // moved after a reader had already read it, so its readers still hold
        // the pre-spill value. Full mode heals that on its next unconditional
        // pass; incremental would serve the stored values forever and land one
        // pass behind, so this pass is full too. Consumed here, and set again
        // below if the full pass leaves debt of its own.
        let convergence_debt = self.graph.take_convergence_debt();
        // Debt alone forces the pass full. When that pass also carries pending
        // edits, the cell diff below cannot see them (a user write lands before
        // evaluate, so it is already in the "before" snapshot), and a delta that
        // silently drops the edit is worse than reporting Everything.
        let debt_over_pending_edits = convergence_debt && !self.graph.should_recompute_full();
        if convergence_debt || self.graph.should_recompute_full() {
            // A full from a shape-changing edit or the first pass may change any
            // cell, so drop the delta. A trailing delete can leave dirty empty
            // (nothing below to shift) while still emptying cells; catch that
            // before the CF-diff branch, which would run evaluate_full, clear
            // the flag in after_pass, and report Cells([]).
            // A redundant full with nothing pending keeps the delta, unless
            // RAND/NOW/TODAY are present: a full pass re-rolls those.
            // OFFSET does not re-roll and must not wipe the delta.
            if debt_over_pending_edits
                || self.graph.take_structural_unknown()
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
        let mut always_report: Vec<Position> = write_seeds.into_iter().collect();
        always_report.extend(always_dirty.iter().copied());
        for &cell in &always_dirty {
            self.graph.mark_dirty(cell);
        }
        let (seeds, affected) = self.graph.take_seeds_and_affected();
        // A wide-fanout edit reaches most of the workbook, where incremental
        // bookkeeping costs about as much as it saves; past half the formulas a
        // full pass is cheaper. The floor keeps small workbooks on the fast path.
        // Verify skips this: it is a performance fallback, not a correctness one.
        if self.formula_count_stale {
            self.recount_formula_cells();
        }
        if self.should_fallback_fanout(affected.len()) {
            self.evaluate_full_untracked();
            return EvalPass::Full;
        }
        // Array and spill cells need the full pass's two-phase ordering.
        // Parse-time dynamic formulas (`=SEQUENCE`, `=E15#`) are ArrayFormula
        // before the first eval, but are not in `graph.arrays` until we see
        // them; a fresh or re-dirtied dynamic anchor is a seed, so seeds are
        // checked for anchors directly. An already-evaluated dynamic anchor
        // whose last result was 1x1 has no spill cells, is not in `arrays`,
        // and behaves as a scalar (`=LET(..)`, a called LAMBDA, `=INDEX(..)`);
        // it stays incremental. If its result grows during the pass, the
        // post-pass arrays comparison below falls back to Full.
        if affected.iter().any(|cell| self.graph.arrays.contains(cell))
            || seeds.iter().any(|cell| self.is_dynamic_array_anchor(*cell))
        {
            self.evaluate_full_untracked();
            return EvalPass::Full;
        }
        // Per-evaluation scratch. `support` feeds only the full pass's spill
        // ordering and is rebuilt there; `variable_stack` and `lambdas` are
        // repopulated by the formulas that evaluate below. Without these clears
        // the scratch of every historical pass accumulates for the lifetime of
        // the model. `links` must persist: out-of-cone HYPERLINK cells keep
        // theirs, and `change_key` reads it.
        self.support.clear();
        self.clear_variable_stack();
        self.clear_lambdas();
        // Recompute the affected cells and collect the ones whose value actually
        // moved. A cycle in the affected set has no topological order, so fall
        // back to recomputing the whole set, where `evaluate_cell`'s recursion
        // still reports `#CIRC!`.
        self.saw_circular_reference = false;
        let (changed, cycle_was_known) = match self.graph.topo_order(&affected) {
            Some(order) => (
                self.recompute_frontier(&affected, &seeds, &always_report, &order, HashSet::new()),
                false,
            ),
            None => (self.recompute_all(&affected, &always_report), true),
        };
        // A cycle the cone did not know about: the closing edge is only observed
        // while the pass runs, so the ordering above was computed on a graph that
        // did not have it and `#CIRC!` landed on a different member than Full's
        // recursion picks. Full sees the cycle on this same pass, so redo the
        // pass as full rather than land one evaluate behind. An already-known
        // cycle went through `recompute_all`, which walks the cone in Full's own
        // row-major order and places `#CIRC!` the same way.
        if self.saw_circular_reference && !cycle_was_known {
            self.evaluate_full_untracked();
            return EvalPass::Full;
        }
        // An evaluation write changed an array footprint: a spill landed, a
        // CSE range filled, or an anchor stored #SPILL!. Fall back to Full so
        // spill dependents are not missed and `collect_array_cells` rebuilds
        // the index exactly.
        if self.wrote_array_cells {
            self.evaluate_full_untracked();
            return EvalPass::Full;
        }
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
        affected: &HashSet<Position>,
        must_run: &[Position],
        always_report: &[Position],
        order: &[Position],
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
        for &position in affected {
            self.cells.remove(&position);
        }
        for &position in order {
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
        for &position in affected {
            if let std::collections::hash_map::Entry::Vacant(entry) = self.cells.entry(position) {
                if let Some(state) = saved.get(&position) {
                    entry.insert(state.clone());
                }
            }
        }
        self.recompute_scope = None;
        // OFFSET/INDIRECT can recompute a helper via evaluate_cell before this
        // loop reaches it; that cell never entered `stale`.
        for &position in affected {
            if report.contains(&position) || self.change_key(position) != before[&position] {
                changed.insert(position);
            }
        }
        changed.into_iter().collect()
    }

    /// Recomputes the whole affected set, used when a cycle prevents ordering.
    /// Returns `always_report` plus every other cell whose value moved.
    ///
    /// With no topological order the walk order is what decides which member of
    /// a cycle `evaluate_cell`'s recursion enters first, and so where `#CIRC!`
    /// lands. That has to be Full's order, and Full's order is two phases, not
    /// one: `evaluate_full` runs `collect_spill_cells` (every
    /// `Cell::ArrayFormula`, row-major) and evaluates those before walking the
    /// rest of the workbook row-major. The cone is walked the same way: array
    /// formulas first, then the rest, each row-major. Anything with a real
    /// spill footprint took the arrays→Full fallback before reaching here, so
    /// the only anchors left are scalar-result (1x1) ones; they write no
    /// members, which is why phase 1's spill-order correction has nothing to do.
    fn recompute_all(
        &mut self,
        affected: &HashSet<Position>,
        always_report: &[Position],
    ) -> Vec<Position> {
        let mut order: Vec<Position> = affected.iter().copied().collect();
        order.sort_unstable();
        let before: HashMap<Position, Option<ChangeKey>> =
            order.iter().map(|&p| (p, self.change_key(p))).collect();
        for &position in &order {
            self.invalidate(position);
        }
        let (mut walk, rest): (Vec<Position>, Vec<Position>) = order
            .iter()
            .partition(|&&position| self.is_array_formula(position));
        walk.extend(rest);
        self.recompute_scope = Some(affected.clone());
        for (sheet, row, column) in walk {
            self.evaluate_cell(CellReferenceIndex { sheet, row, column });
        }
        self.recompute_scope = None;
        let report: HashSet<Position> = always_report.iter().copied().collect();
        order
            .into_iter()
            .filter(|p| report.contains(p) || self.change_key(*p) != before[p])
            .collect()
    }

    /// Whether `position` holds an array formula that has never been evaluated.
    /// It is in the array index precisely because its extent is unknown, and it
    /// has no pre-pass value to compare against: every first evaluation would
    /// look like a mid-pass move.
    fn is_unevaluated_array(&self, (sheet, row, column): Position) -> bool {
        matches!(
            self.workbook
                .worksheet(sheet)
                .ok()
                .and_then(|ws| ws.cell(row, column)),
            Some(Cell::ArrayFormula {
                v: crate::types::FormulaValue::Unevaluated,
                ..
            })
        )
    }

    /// Whether `position` holds an array formula of either kind. This is
    /// exactly `collect_spill_cells`'s phase-1 membership test, so that
    /// `recompute_all` can reproduce the full pass's two-phase walk order.
    fn is_array_formula(&self, (sheet, row, column): Position) -> bool {
        matches!(
            self.workbook
                .worksheet(sheet)
                .ok()
                .and_then(|ws| ws.cell(row, column)),
            Some(Cell::ArrayFormula { .. })
        )
    }

    /// Parse-time dynamic-array anchors (`ArrayKind::Dynamic`) need the Full
    /// two-phase spill order even before they appear in `graph.arrays`.
    fn is_dynamic_array_anchor(&self, (sheet, row, column): Position) -> bool {
        matches!(
            self.workbook
                .worksheet(sheet)
                .ok()
                .and_then(|ws| ws.cell(row, column)),
            Some(Cell::ArrayFormula {
                kind: ArrayKind::Dynamic,
                ..
            })
        )
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
        // The array footprint's values entering the pass. Full's two phases are
        // not a fixed point: a formula outside phase 1 can read a spill member
        // before the anchor refills it (after a move, a delete, or a first
        // spill), and only Full's next unconditional pass heals that reader.
        // This is a snapshot taken across `evaluate_full`, not a lazy view: the
        // comparison below is against the values as they were before the pass,
        // so the iterator cannot be fused into it.
        let footprint_before: Vec<(Position, Option<ChangeKey>)> = before
            .iter()
            .filter(|&&p| !self.is_unevaluated_array(p))
            .map(|&p| (p, self.change_key(p)))
            .collect();
        self.saw_circular_reference = false;
        self.evaluate_full();
        // A cycle that runs through the array (a member read while its anchor is
        // still evaluating) resolves against the member's pre-pass value, so a
        // footprint that then moves leaves the reader holding the old one even
        // when no edge records the read.
        let circular = self.saw_circular_reference;
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
        // A footprint cell that moved this pass and that something read this
        // pass may have been read before it moved. The reads are exactly the
        // edges the pass just recorded, so a dependent means a reader exists.
        // Conservative by one pass at worst: if the reader in fact read after
        // the write, the forced full pass moves nothing and clears the debt.
        let debt = footprint_before.iter().any(|(p, was)| {
            self.change_key(*p) != *was && (circular || !self.graph.dependents_of(*p).is_empty())
        });
        if debt {
            self.graph.note_convergence_debt();
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
