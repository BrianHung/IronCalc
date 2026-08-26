//! The `RecalcMode::Verify` oracle: the checks that hold incremental to what
//! full produces.
//!
//! Compiled only under the `recalc_verify` feature, so none of it ships. The
//! module runs an incremental pass and then asserts three things about it: the
//! delta names every cell whose observable state moved and nothing else, every
//! stored formula value equals a live re-evaluation, and a shadow full pass on
//! the same state agrees cell for cell. The shadow pass runs on a snapshot that
//! is restored afterwards, so the check cannot repair the state it is checking.

use std::collections::{HashMap, HashSet};

use super::incremental::ChangeKey;
use super::{ChangedCells, Model};
use crate::cf_types::CfCellResult;
use crate::dependency_graph::Position;
use crate::expressions::types::CellReferenceIndex;
use crate::model::incremental::EvalPass;
use crate::types::Cell;

/// Every cell's full observable state (`ChangeKey` plus conditional format),
/// used by the `Verify` check to compare incremental against full.
type RenderSnapshot = HashMap<Position, (Option<ChangeKey>, Vec<CfCellResult>)>;

impl Model<'_> {
    /// When the pass stayed Incremental, runs a full pass and asserts they agree,
    /// and that the recorded delta names every cell whose observable state moved.
    /// A Full fallback has nothing to compare. Backs
    /// [`RecalcMode::Verify`](crate::dependency_graph::RecalcMode::Verify).
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
    fn assert_stored_matches_live(&mut self) {
        // The cells that never serve a stored value are exactly the cells this
        // check cannot make: a volatile re-rolls, and a cell whose last result
        // was not a function value -- on a cycle, downstream of one, or reading
        // a blocked anchor -- holds something a one-cell scratch frame reading
        // the store does not reproduce.
        let mut skip = self.graph.always_dirty_cells();
        skip.extend(self.graph.never_served().iter().copied());
        skip.extend(self.graph.blocked_array_readers().iter().copied());
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
                self.cell_at(position),
                Some(Cell::ArrayFormula { .. } | Cell::SpillCell { .. })
            ) {
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
}

/// Unions two change records. `All` wins; otherwise the cells are merged.
fn merge_changed_cells(consumer: ChangedCells, this_pass: ChangedCells) -> ChangedCells {
    match (consumer, this_pass) {
        (ChangedCells::All, _) | (_, ChangedCells::All) => ChangedCells::All,
        (ChangedCells::Delta(mut a), ChangedCells::Delta(b)) => {
            a.extend(b);
            ChangedCells::Delta(a)
        }
    }
}
