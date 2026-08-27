//! The incremental scheduler: which cells a pass recomputes, in what order,
//! and when it gives up and runs a full pass instead.
//!
//! Edges are the reads recorded while a formula evaluates. A pass recomputes
//! only the cells reachable from those that changed (plus the formulas that
//! read RAND/NOW/TODAY), stopping wherever a recomputed value turns out
//! unchanged. Everything it cannot model is answered by falling back to full,
//! so the fallbacks here are the list of what incremental does not handle.
//!
//! What counts as unchanged, and the delta the pass records, are
//! [`super::changed_cells`]. The array index it consults is
//! [`super::array_index`]. Of the two sets of untrustworthy stored values,
//! the readers of blocked spill anchors are rebuilt by
//! [`super::unstable_cells`]; the cycle cone needs no cell state, so the graph
//! derives it and this module installs it at the end of a selective pass (as
//! `evaluate_full` does at the end of a full one). The oracle that checks the
//! whole thing is `super::verify`.

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

/// How many two-phase passes one `evaluate` may run while settling. Every shape
/// found so far settles in two; the bound turns a workbook that does not into a
/// loud debug assertion instead of a spin. See
/// [`Model::evaluate_full_to_fixed_point`].
const MAX_SETTLING_PASSES: usize = 4;

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
                let before = self.workbook_change_keys();
                let cf_before = self.cf_cache.clone();
                self.evaluate_full_to_fixed_point();
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
        // Selectivity is earned, not assumed: this pass is selective only if
        // every cell it would touch is one a selective pass can model.
        if !self.cone_is_plain(&affected) {
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
                self.recompute_frontier(&affected, &seeds, &always_report, &order),
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
        //
        // Read `cycle_was_known` as "some cycle was already open", not "this
        // cycle was": `never_served` seeds every known cycle dirty on every
        // pass, so a known cycle is in every cone and `topo_order` fails on
        // every cone. A cycle closing for the first time while another one is
        // open therefore does not reach this redo at all -- it took the `Err`
        // arm above. That is still correct, and it is why the arm can be
        // trusted rather than tightened: `recompute_all` walks the cone in
        // Full's own two phases (array formulas first, then the rest, each
        // row-major), which is the order that decides where `#CIRC!` lands, so
        // the new cycle gets Full's placement without a redo. The redo exists
        // only for the case `recompute_all` never ran: a clean `topo_order`
        // whose walk then hit a cycle anyway.
        if self.saw_circular_reference && !cycle_was_known {
            self.evaluate_full_untracked();
            return EvalPass::Full;
        }
        // The one hazard plainness cannot rule out in advance. It admits a
        // dynamic anchor whose last result was a plain 1x1 scalar, because
        // `=LET(..)`, a called `LAMBDA` and `=INDEX(..)` are stored that way and
        // must not cost a full pass. Whether *this* pass's result is still 1x1
        // is not a property of the stored cell: it is what the pass produces. An
        // anchor that grows spills members, and this is where that is found out.
        // Redo the pass as full so spill dependents are not missed and
        // `collect_array_cells` rebuilds the index exactly.
        if self.wrote_array_cells {
            self.evaluate_full_untracked();
            return EvalPass::Full;
        }
        // Rebuild the set from the graph this pass recorded. The cone is
        // closed under dependents, and every cell that could have gained or
        // lost an edge re-evaluated inside it, so a cycle that formed or broke
        // is visible here and cells outside the cone cannot have changed
        // status. When the cone ordered cleanly and no `#CIRC!` was reported
        // there is no cycle to find. A blocked anchor cannot appear here at
        // all: it is in the array index, so a cone reaching one left through
        // the Full fallback.
        let cycle_cone = if cycle_was_known || self.saw_circular_reference {
            self.graph.cycle_cone(&affected)
        } else {
            HashSet::new()
        };
        self.graph.set_never_served(cycle_cone);
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

    /// Whether `cone` is **plain**: every cell in it is one a selective pass can
    /// model, so this pass may be selective. Anything else runs the full pass.
    ///
    /// This is the whole of the scheduling decision that is about the cone. It
    /// is a whitelist on purpose. The engine used to carry a blacklist of
    /// hazards -- arrays in the cone, a dynamic anchor among the seeds, a
    /// blocked reader, an evaluation write into a footprint found out about
    /// afterwards -- and a hazard nobody had thought of was a wrong value.
    /// Inverted, a case nobody thought of fails the predicate and costs a full
    /// pass: a missed case degrades to slow rather than to wrong.
    ///
    /// The clauses, each with its own witness:
    ///
    /// - **P1, no array in the cone.** No cone member is in `graph.arrays`,
    ///   which holds every anchor and every spill member. Spilling needs the
    ///   full pass's two-phase ordering, and a spill member's value is its
    ///   anchor's output rather than its own. This is the index rather than the
    ///   cells, deliberately: it is what sees a *ghost* member -- a declared
    ///   footprint position whose spill cell a structural edit dropped, where
    ///   the live cell no longer says "array" but the anchor still owns the
    ///   position. It cannot miss a live one either, because
    ///   [`array_footprint`](super::array_index::array_footprint) is the single
    ///   definition of what goes in and every array cell reaches it: a
    ///   user-written one through the journal drain, an evaluation-written one
    ///   through the `wrote_array_cells` redo below, and a moved one through
    ///   `shift`. A dynamic anchor whose last result was a plain 1x1 scalar is
    ///   not in the index and is plain: `=LET(..)`, a called `LAMBDA`,
    ///   `=INDEX(..)` are everyday formulas and must not cost a full pass.
    ///   Whether such an anchor's result is *still* 1x1 is not a property of any
    ///   stored state, so that one hazard survives the inversion as a post-pass
    ///   redo on `wrote_array_cells`; see `evaluate_selective`.
    /// - **P2, trust.** No cone member is a reader of a blocked spill anchor.
    ///   Such a reader's stored value came from the live array's top-left, not
    ///   from the anchor's stored `#SPILL!`, so recomputing it here would read
    ///   the error instead. Only the full pass evaluates the anchor live.
    ///
    /// Three clauses the predicate deliberately does **not** have, each because
    /// it would cost a case incremental wins today and buys no correctness:
    ///
    /// - *No cone member in `never_served`.* A known cycle is seeded dirty on
    ///   every pass, so it is in every cone, so this clause would send every
    ///   workbook containing one cycle to a full pass forever. The cone with a
    ///   cycle in it is handled instead by walking it in Full's own two phases
    ///   (`recompute_all`), which is what decides where `#CIRC!` lands.
    /// - *No structural op this drain.* Row and column *moves* already force the
    ///   graph to rebuild, which the readiness gate above catches. Inserts and
    ///   deletes shift the indices in place and stay selective by design, and
    ///   the clause would throw that away.
    /// - *No volatile beyond the seeded always-dirty.* There is no such thing to
    ///   exclude: volatility is a recorded `Input`, every reader of one is in
    ///   `always_dirty_cells`, and this pass seeded all of them.
    ///
    /// The fanout budget is checked separately and before this, because it is a
    /// performance choice rather than a statement about what can be modelled --
    /// which is why `Verify` disables that one and not this.
    fn cone_is_plain(&self, cone: &HashSet<Position>) -> bool {
        cone.iter().all(|position| {
            !self.graph.arrays.contains(position)
                && !self.graph.blocked_array_readers().contains(position)
        })
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
    ) -> Vec<Position> {
        let before = self.change_keys(affected.iter().copied());
        self.recompute_scope = Some(affected.clone());
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
        self.evaluate_full_to_fixed_point();
        self.changed_cells = ChangedCells::All;
    }

    /// Runs the two-phase pass until the workbook settles, and leaves a newly
    /// observed array anchor dirty.
    ///
    /// **One two-phase pass is not a fixed point.** Phase 1 spills the arrays
    /// and phase 2 evaluates the rest, but `evaluate_cell` recurses, so a
    /// phase-2 formula can be pulled in early and read a footprint position
    /// before its anchor refills it -- after a row move, a delete, or a first
    /// spill. That reader then holds a value the same inputs would never
    /// produce again, and only a *further* whole-workbook pass repairs it.
    ///
    /// A single `evaluate` therefore has to run that further pass itself.
    /// Excel settles fully per recalculation, so this is what a caller expects,
    /// and it is what makes `evaluate` a function of the workbook's inputs
    /// rather than of how many times it has been called. The engine used to
    /// reproduce the unsettled window instead: the full pass recorded
    /// *convergence debt* and the next pass healed it, so `Incremental` matched
    /// `Full`'s accident pass for pass. Both sides of that agreement are gone;
    /// see "Intentional divergences" in `base/src/recalc/README.md`.
    ///
    /// The re-run condition is the one the debt flag used to defer: this pass
    /// moved an array footprint. The reader half of the old condition is not
    /// repeated here, deliberately -- edges only exist in the tracing modes, so
    /// a reader test would settle `Incremental` and leave `Full` unsettled,
    /// which is the one divergence this engine may not have. An extra pass over
    /// a footprint nothing read recomputes the same values and stops.
    ///
    /// **Termination.** Iteration *k*+1 runs the same deterministic pass over
    /// iteration *k*'s output, and stops as soon as the footprint it was handed
    /// comes back unchanged. A re-run only fires when the previous pass wrote a
    /// *different* value into a footprint position, which happens only where an
    /// anchor's inputs or extent moved under it; the anchor's own inputs are
    /// settled by the pass that moved them, so each re-run resolves one layer
    /// of anchor-reads-anchor and the cascade is bounded by the depth of that
    /// chain. Two passes is what every shape found so far needs. The bound
    /// below is the belt: a workbook that has not settled by then is reported,
    /// and in release the pass stops rather than spins -- identically in both
    /// modes, so the modes still agree.
    fn evaluate_full_to_fixed_point(&mut self) {
        let arrays_at_entry = self.graph.arrays.snapshot();
        let mut settled = false;
        for _ in 0..MAX_SETTLING_PASSES {
            // The array footprint's values entering this pass. `change_keys`
            // takes the snapshot eagerly, which is the point: the comparison
            // below is against the values as they were before the pass, so this
            // must not become a lazy view of the post-pass state.
            let before = self.graph.arrays.snapshot();
            let footprint_before = self.change_keys(
                before
                    .iter()
                    .copied()
                    .filter(|&p| !self.is_unevaluated_array(p)),
            );
            self.evaluate_full();
            if footprint_before
                .iter()
                .all(|(p, was)| self.change_key(*p) == *was)
            {
                settled = true;
                break;
            }
        }
        debug_assert!(
            settled,
            "evaluate_full did not settle in {MAX_SETTLING_PASSES} passes: an array footprint \
             is still moving under its readers. The workbook is left as the last pass wrote it."
        );
        // A newly observed dynamic-array anchor may not have seen a SEQUENCE
        // that spilled later in the same pass (`E15#` after `=SEQUENCE(3)`).
        // Leave it dirty so the next evaluate takes the arrays->Full path.
        let new: Vec<Position> = self
            .graph
            .arrays
            .snapshot()
            .into_iter()
            .filter(|p| !arrays_at_entry.contains(p))
            .collect();
        for p in new {
            self.graph.mark_dirty(p);
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
