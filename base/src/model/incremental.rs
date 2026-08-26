//! Incremental recalculation: observed-read graph, selective evaluation, and
//! the changed-cell delta it exposes.
//!
//! Edges are the reads recorded while a formula evaluates. The incremental
//! pass recomputes only the cells reachable from those that changed (plus
//! formulas that read RAND/NOW/TODAY) and records which ones moved, so
//! [`Model::take_changed_cells`] can report a precise delta.

use std::collections::{HashMap, HashSet};

use super::changed_cells::{record_snapshot_diff, ChangedCells};
use crate::dependency_graph::Position;
#[cfg(feature = "recalc_verify")]
use crate::dependency_graph::RecalcMode;
use crate::expressions::types::CellReferenceIndex;
use crate::model::{is_phase_one_cell, Model};

/// Below this many formula cells, incremental never falls back on fanout: the
/// bookkeeping is cheap in absolute terms and full has no edge to exploit.
const INCREMENTAL_FANOUT_FLOOR: usize = 1024;

/// Fall back to a full pass once an edit's fanout reaches this fraction of the
/// formula cells: `2` means half, past which full is cheaper than the
/// incremental bookkeeping it would save.
const INCREMENTAL_FANOUT_RATIO: usize = 2;

/// Whether an evaluate stayed incremental or fell back to a full pass.
pub(crate) enum EvalPass {
    Incremental,
    Full,
}

impl Model<'_> {
    /// Recomputes the workbook incrementally, or decides it cannot and runs a
    /// full pass instead. Returns which of the two happened.
    ///
    /// The guarantee is that the workbook afterwards holds what a full pass
    /// would have produced from the same state, pass for pass -- not merely
    /// eventually. Every case the incremental path cannot model is answered by
    /// falling back rather than approximating, so the fallbacks below are the
    /// enumeration of what it cannot model.
    ///
    /// Requires that the write journal has already been drained into the graph,
    /// which `Model::evaluate` does before calling. Leaves the graph ready and
    /// its dirty set empty; the delta accumulates into `changed_cells` until a
    /// consumer takes it.
    pub(crate) fn evaluate_selective(&mut self) -> EvalPass {
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
                let before = self.workbook_change_keys();
                let cf_before = self.cf_cache.clone();
                self.evaluate_full_and_follow_up_new_arrays();
                let after = self.workbook_change_keys();
                record_snapshot_diff(&mut self.changed_cells, &before, &after);
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
        // Cells whose last result was not a genuine function value never serve
        // it back: a cycle has no fixed point, so what its members and their
        // readers hold is an artifact of where the cycle was entered, and a
        // blocked anchor's `#SPILL!` is not what its readers saw. A full pass
        // re-derives all of that from scratch every time; seeding them dirty
        // makes this pass do the same, and pulls their readers into the cone
        // with them. They are not `always_report`: the delta still names only
        // the cells whose value actually moved.
        let never_served: Vec<Position> = self.graph.never_served().iter().copied().collect();
        for &cell in &never_served {
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
        // A reader of a blocked anchor is in the same position: its value came
        // from the live array's top-left, not from the anchor's stored
        // `#SPILL!`, so recomputing it here would read the error instead. Only
        // the full pass evaluates the anchor live.
        if affected
            .iter()
            .any(|cell| self.graph.blocked_array_readers().contains(cell))
        {
            self.evaluate_full_untracked();
            return EvalPass::Full;
        }
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
            Ok(order) => (
                self.recompute_frontier(&affected, &seeds, &always_report, &order, HashSet::new()),
                false,
            ),
            Err(_) => (self.recompute_all(&affected, &always_report), true),
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
        // Rebuild the set from the graph this pass recorded. The cone is
        // closed under dependents, and every cell that could have gained or
        // lost an edge re-evaluated inside it, so a cycle that formed or broke
        // is visible here and cells outside the cone cannot have changed
        // status. When the cone ordered cleanly and no `#CIRC!` was reported
        // there is no cycle to find, and only the per-cell witnesses can add
        // anything. A blocked anchor cannot appear here at all: it is in the
        // array index, so a cone reaching one left through the Full fallback.
        let cycle_cone = if cycle_was_known || self.saw_circular_reference {
            self.graph.cycle_cone(&affected)
        } else {
            HashSet::new()
        };
        self.refresh_unstable_cells(cycle_cone, &affected);
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
        let before = self.change_keys(affected.iter().copied());
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
            if self.reports_change(position, &before, &report) {
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
            if self.reports_change(position, &before, &report) {
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
    /// lands. That has to be Full's order, and a walk of the cone reproduces
    /// it: a cell that can recurse into a cycle member reads it, transitively,
    /// so it is a reader of a never-served cell and therefore in the cone. Full
    /// reaches no cycle member through a cell this walk does not also have.
    ///
    /// Full's order is two phases, not one: `evaluate_full` runs
    /// `collect_spill_cells` (every `Cell::ArrayFormula`, row-major) and
    /// evaluates those before walking the rest of the workbook row-major. The
    /// cone is walked the same way: array formulas first, then the rest, each
    /// row-major. Anything with a real spill footprint took the arrays→Full
    /// fallback before reaching here, so the only anchors left are
    /// scalar-result (1x1) ones; they write no members, which is why phase 1's
    /// spill-order correction has nothing to do. What phase 1 still decides is
    /// the entry point, for an anchor a cycle can reach that is not itself a
    /// seed -- the pass a cycle first closes around one, say.
    fn recompute_all(
        &mut self,
        affected: &HashSet<Position>,
        always_report: &[Position],
    ) -> Vec<Position> {
        let mut order: Vec<Position> = affected.iter().copied().collect();
        order.sort_unstable();
        let before = self.change_keys(order.iter().copied());
        for &position in &order {
            self.invalidate(position);
        }
        let walk = self.in_full_pass_order(&order);
        self.recompute_scope = Some(affected.clone());
        for (sheet, row, column) in walk {
            self.evaluate_cell(CellReferenceIndex { sheet, row, column });
        }
        self.recompute_scope = None;
        let report: HashSet<Position> = always_report.iter().copied().collect();
        order
            .into_iter()
            .filter(|&position| self.reports_change(position, &before, &report))
            .collect()
    }

    /// `positions`, which must already be in `(sheet, row, column)` order,
    /// rearranged the way [`Model::evaluate_full`] walks the workbook: the
    /// phase-1 cells first, then the rest, each still in that order.
    ///
    /// The full pass gets that order from `collect_spill_cells` followed by
    /// `get_all_cells`, which walks the whole workbook; this walks one cone.
    /// The inputs differ, so the traversals stay separate, but both select
    /// phase 1 with [`is_phase_one_cell`], which is where the agreement lives.
    fn in_full_pass_order(&self, positions: &[Position]) -> Vec<Position> {
        let (mut phase_one, rest): (Vec<Position>, Vec<Position>) = positions
            .iter()
            .copied()
            .partition(|&position| self.cell_at(position).is_some_and(is_phase_one_cell));
        phase_one.extend(rest);
        phase_one
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
        // `change_keys` takes the snapshot eagerly, which is the point: the
        // comparison below is against the values as they were before the pass,
        // so this must not become a lazy view of the post-pass state.
        let footprint_before = self.change_keys(
            before
                .iter()
                .copied()
                .filter(|&p| !self.is_unevaluated_array(p)),
        );
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
