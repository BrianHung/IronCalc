//! The incremental scheduler: which cells a pass recomputes, in what order,
//! and when it gives up and runs a full pass instead.
//!
//! Edges are the reads recorded while a formula evaluates. A pass recomputes
//! only the cells reachable from those that changed (plus the formulas that
//! read RAND/NOW/TODAY), stopping wherever a recomputed value turns out
//! unchanged -- but only when `cone_is_plain` says the cone is one a selective
//! pass can model. The default is the full pass, so what this module does not
//! handle costs time rather than correctness.
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
use crate::dependency_graph::{Position, RecalcMode};
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

/// How many full passes the scheduler must *choose* in a row before it will
/// stop paying for tracing.
///
/// Chosen, not merely full, and that is the whole of the evidence test. A
/// rebuild — the graph was not ready, so full was the only pass on offer — says
/// nothing about whether tracing pays, and the pass after one is commonly the
/// pass that spends what it recorded. A pass that had a ready graph and a cone
/// to walk and went full anyway is a different fact: the investment the last
/// pass made is being declined, in front of us.
///
/// Twelve, and the number comes from the differential fuzzer rather than from
/// the bench. Untracing a pass saves a fraction of a pass and costs a whole one
/// when the guess is wrong, and the fuzzer prices exactly that: it holds itself
/// to a floor of half its non-volatile evaluates staying selective, or the
/// oracle is comparing `Full` against `Full`. At six the floor is missed on one
/// of its configurations; at twelve the fuzzer's selectivity is within a point
/// and a half of an engine with no hysteresis at all, and — because the arming
/// passes are a smaller and smaller share of a long run — the long-run cost is
/// the same either way. Cheap insurance, so buy plenty.
const CHOSEN_BEFORE_ACTING: u32 = 12;

/// The longest stretch of untraced passes the backoff will reach.
///
/// The stretch doubles for every stretch the run survives, so a long run pays
/// the investment a logarithmic number of times and the longer it lasts the
/// closer it costs to `Full`'s price. The cap is what stops that becoming an
/// unbounded blind spell: a run that has lasted long enough to reach it waits
/// at most this many passes to find out it is over.
const MAX_UNTRACED_STRETCH: u32 = 16;

/// Whether an evaluate stayed incremental or fell back to a full pass.
pub(crate) enum EvalPass {
    Incremental,
    Full,
}

/// Whether a full pass was picked over a selective pass that was there to be
/// run, or was the only pass on offer. Named rather than a bare `bool` because
/// the call sites are five one-word arguments and the distinction is what
/// [`FullPassRun`] counts.
#[derive(Clone, Copy)]
enum Chosen {
    /// The graph was ready and the cone was there to be walked; this pass went
    /// full anyway.
    Yes,
    /// The graph was not ready, or nothing was dirty: there was no selective
    /// pass to prefer.
    No,
}

/// The cost contract's whole state: where in a run of full passes this model
/// is.
///
/// A traced full pass costs `Full`'s pass plus an investment — the read tracing
/// and the graph rebuild — and that investment buys one thing, the *next*
/// pass's selectivity. It is a good trade when the next pass can spend it and a
/// bad one when the next pass is full too, so on a run of full passes the
/// engine stops paying it.
///
/// What keeps that from being a licence to guess is the shape of the bet.
/// Untracing saves a fraction of a pass and costs a whole one when it is wrong,
/// because an untraced pass leaves the graph unready and the pass after it has
/// no graph to be selective with. So the run has to prove itself at length
/// first ([`Self::Watching`], and only *chosen* full passes count), and is then
/// acted on gently: the first untraced stretch is one pass, and every stretch
/// the run survives doubles the next, up to [`MAX_UNTRACED_STRETCH`]. A run
/// that ends early costs one wasted pass; a run that goes on pays the
/// investment a logarithmic number of times.
///
/// See `base/src/recalc/README.md`, "The cost contract".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FullPassRun {
    /// No run: the previous pass was selective, was a rebuild, or there has not
    /// been one yet. The next full pass traces.
    #[default]
    NoRun,
    /// Full passes the scheduler chose in a row, still too few to act on. Every
    /// one of them traces, which is what keeps a workbook that falls back once
    /// and then goes on being selective — an array deleted, one wide edit among
    /// narrow ones — selective on the very next pass.
    Watching(u32),
    /// A run long enough to act on: `untraced` passes have run untraced out of
    /// the current `stretch`, and the stretch doubles each time the run
    /// survives one. Once here, a rebuild continues the run rather than ending
    /// it — the untraced pass is what made the graph unready in the first
    /// place.
    Running { untraced: u32, stretch: u32 },
}

impl FullPassRun {
    /// Whether the full pass about to run records what it reads.
    fn traces(&self) -> bool {
        !matches!(self, Self::Running { untraced, stretch } if untraced < stretch)
    }

    /// Folds one full pass into the run. `chosen` is whether the scheduler
    /// picked full over a selective pass that was available to it.
    fn record(&mut self, traced: bool, chosen: bool) {
        *self = match (*self, chosen) {
            (Self::Running { untraced, stretch }, _) if !traced => Self::Running {
                untraced: untraced + 1,
                stretch,
            },
            (Self::Running { stretch, .. }, _) => Self::Running {
                untraced: 0,
                stretch: (stretch * 2).min(MAX_UNTRACED_STRETCH),
            },
            (_, false) => Self::NoRun,
            (Self::Watching(seen), true) if seen + 1 >= CHOSEN_BEFORE_ACTING => Self::Running {
                untraced: 0,
                stretch: 1,
            },
            (Self::Watching(seen), true) => Self::Watching(seen + 1),
            (Self::NoRun, true) => Self::Watching(1),
        };
    }

    /// Ends the run: the pass that just finished was selective, so the graph is
    /// live and being spent, and the next full pass is worth tracing again.
    fn ended(&mut self) {
        *self = Self::NoRun;
    }
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
                self.fall_back_to_full(Chosen::No);
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
            self.fall_back_to_full(Chosen::Yes);
            return EvalPass::Full;
        }
        // Selectivity is earned, not assumed: this pass is selective only if
        // every cell it would touch is one a selective pass can model.
        if !self.cone_is_plain(&affected) {
            self.fall_back_to_full(Chosen::Yes);
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
            self.fall_back_to_full(Chosen::Yes);
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
            self.fall_back_to_full(Chosen::Yes);
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
        self.full_pass_run.ended();
        EvalPass::Incremental
    }

    /// Runs the full pass this scheduler could not avoid, and charges it
    /// against the cost contract. [`FullPassRun`] decides whether it records
    /// what it reads; `chosen` is the evidence that decision is made from.
    ///
    /// An untraced pass runs as `RecalcMode::Full` runs it, *by being* it for
    /// the duration. The mode is the one thing every recording site already
    /// consults — [`Model::tracing`], the array-anchor edge in `evaluate_cell`,
    /// `commit_reads`, and the tail of `evaluate_full` that chooses between
    /// marking the graph ready and forcing a rebuild — so borrowing it is what
    /// makes "exactly Full's cost" true by construction rather than by a second
    /// list of gates somebody has to keep in step with the first. That tail is
    /// also what leaves the graph unready, so an untraced pass can no more
    /// serve a stale edge than a `Full`-mode one can.
    ///
    /// The delta is `Everything` either way, which is already what a fallback
    /// reports.
    fn fall_back_to_full(&mut self, chosen: Chosen) {
        let traced = self.fallback_traces();
        if traced {
            self.evaluate_full_reporting_everything();
        } else {
            self.as_full_mode().evaluate_full_reporting_everything();
        }
        self.full_pass_run
            .record(traced, matches!(chosen, Chosen::Yes));
    }

    /// Whether the fallback about to run traces. `Verify` disables the
    /// hysteresis exactly as it disables the fanout guard, and for the same
    /// reason: both are performance choices, and the oracle's whole job is to
    /// compare a *selective* pass against a shadow full one. An untraced pass
    /// leaves no graph for the next pass to be selective with, so a Verify run
    /// that honoured the hysteresis would spend most of its passes checking
    /// nothing.
    fn fallback_traces(&self) -> bool {
        #[cfg(feature = "recalc_verify")]
        if self.recalc_mode == RecalcMode::Verify {
            return true;
        }
        self.full_pass_run.traces()
    }

    /// Whether `cone` is **plain**: every cell in it is one a selective pass can
    /// model, so this pass may be selective. Anything else runs the full pass.
    ///
    /// This is the whole of the scheduling decision that is about the cone, and
    /// it is a whitelist rather than a blacklist of hazards on purpose: under a
    /// blacklist a hazard nobody thought of is a wrong value; here a case
    /// nobody thought of is only a full pass.
    ///
    /// - **P1** no cone member is in the array index (I8.5). Spilling needs the
    ///   full pass's two-phase ordering, and a spill member's value is its
    ///   anchor's output rather than its own.
    /// - **P2** no cone member is a reader of a blocked spill anchor (I8.4). Its
    ///   stored value came from the live array's top-left, not from the anchor's
    ///   stored `#SPILL!`, and only the full pass evaluates the anchor live.
    ///
    /// Why P1 reads the index rather than the cells, why three further clauses
    /// are deliberately absent, and the one hazard no pre-pass predicate can see
    /// (the `wrote_array_cells` redo in `evaluate_selective`) are in
    /// `base/src/recalc/README.md`, "When a pass is allowed to be selective".
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
    pub(crate) fn evaluate_full_reporting_everything(&mut self) {
        self.evaluate_full_to_fixed_point();
        self.changed_cells = ChangedCells::All;
    }

    /// The array-footprint positions a pass can be held to: every position the
    /// index names whose anchor has actually been evaluated. An anchor that
    /// never has holds no extent yet, so there is nothing to compare it against.
    ///
    /// Membership, not value. What strands a reader is a footprint position
    /// changing *hands* -- appearing, vanishing, or moving to another anchor --
    /// because reading a live spill member evaluates its anchor first
    /// (`evaluate_cell`'s `SpillCell` arm), so a reader of a member that stayed a
    /// member cannot have been served a pre-write value. A pure value re-roll
    /// under stable membership therefore needs no further pass, which is just as
    /// well: `RANDARRAY` re-rolls every pass by definition, and a value
    /// comparison would ask it to converge to something it has no fixed point
    /// for and spin until the bound.
    fn settled_footprint(&self) -> HashSet<Position> {
        self.graph
            .arrays
            .snapshot()
            .into_iter()
            .filter(|&p| !self.is_unevaluated_array(p))
            .collect()
    }

    /// Runs the two-phase pass until the workbook settles, and leaves a newly
    /// observed array anchor dirty.
    ///
    /// One two-phase pass is not a fixed point: `evaluate_cell` recurses, so a
    /// phase-2 formula can be pulled in early and read a footprint position
    /// before its anchor refills it, and only a further whole-workbook pass
    /// repairs that reader. `evaluate` runs the further pass itself, so what it
    /// returns is the settled state rather than the first approximation of it.
    ///
    /// The re-run condition is [`Model::settled_footprint`] alone, with no test
    /// for whether anything read the moved position: edges exist only in the
    /// tracing modes, so a reader test would settle `Incremental` and leave
    /// `Full` one healing window behind, which is the one divergence this engine
    /// may not have. An extra pass over a footprint nothing read recomputes the
    /// same values and stops.
    ///
    /// Termination, the bound, and the exact extent of the divergence from
    /// pre-engine behaviour are in `base/src/recalc/README.md`, "One `evaluate`
    /// settles" and "Intentional divergences".
    fn evaluate_full_to_fixed_point(&mut self) {
        let arrays_at_entry = self.graph.arrays.snapshot();
        let mut settled = false;
        for _ in 0..MAX_SETTLING_PASSES {
            // The footprint's *membership* entering this pass. Collected eagerly:
            // the comparison below is against the positions as they were before
            // the pass, so this must not become a view of the post-pass index.
            let footprint_before = self.settled_footprint();
            self.evaluate_full();
            if self.settled_footprint() == footprint_before {
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

/// The model with its recalc mode temporarily set to [`RecalcMode::Full`], so
/// that the pass run through it records nothing and costs what a `Full` pass
/// costs.
///
/// A hand-rolled save/set/restore triple would leak the borrowed mode on an
/// early exit and leave an `Incremental` model silently in `Full` for the rest
/// of its life — every later pass correct, none of them ever selective again.
/// The guard makes that unrepresentable: it *is* the mutable handle to the
/// model, so the pass has to run through it, and `Drop` restores the mode on
/// every exit path including a panic. It is the same shape as
/// [`JournalRecordingPaused`](crate::recalc::journal), for the same reason.
#[must_use = "the mode is borrowed only while this guard is alive"]
struct AsFullMode<'a, 'm> {
    model: &'a mut Model<'m>,
    restore: RecalcMode,
}

impl<'m> Model<'m> {
    /// Runs one pass the way `RecalcMode::Full` runs it, whatever mode the
    /// model is in. See [`Model::fall_back_to_full`].
    fn as_full_mode(&mut self) -> AsFullMode<'_, 'm> {
        AsFullMode {
            restore: std::mem::replace(&mut self.recalc_mode, RecalcMode::Full),
            model: self,
        }
    }
}

impl<'m> std::ops::Deref for AsFullMode<'_, 'm> {
    type Target = Model<'m>;

    fn deref(&self) -> &Self::Target {
        self.model
    }
}

impl std::ops::DerefMut for AsFullMode<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.model
    }
}

impl Drop for AsFullMode<'_, '_> {
    fn drop(&mut self) {
        self.model.recalc_mode = self.restore;
    }
}
