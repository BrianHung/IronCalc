//! Dependency graph and recalculation mode for incremental evaluation.
//!
//! The default evaluator recomputes every cell on every edit. This module adds
//! an opt-in incremental path: a forward dependency graph (precedent to
//! dependents) lets [`Model::evaluate`](crate::Model::evaluate) recompute only
//! the cells reachable from the ones that changed.
//!
//! The graph is rebuilt during a full evaluation, from the same reference walk
//! that populates `Model::support`, so it reflects the last full pass. Any change
//! the incremental path does not model (a new or edited formula, a structural
//! edit) forces the next evaluation to be full and rebuild the graph, so
//! incremental is never more than an optimization over what full would produce.

use std::collections::{HashMap, HashSet};

/// Strategy [`Model::evaluate`](crate::Model::evaluate) uses to recompute the
/// workbook, set via [`Model::set_recalc_mode`](crate::Model::set_recalc_mode).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RecalcMode {
    /// Recompute every cell. The default and original behavior.
    #[default]
    Full,
    /// Recompute only the cells reachable from the dirty set.
    Incremental,
    /// Run incremental, then full, and assert they produce identical values.
    Verify,
}

impl RecalcMode {
    /// Reads the mode from `IRONCALC_RECALC` (`full` | `incremental` | `verify`),
    /// defaulting to `Full`, so the test suite can run under a chosen strategy.
    /// wasm has no environment and is always `Full`.
    pub(crate) fn from_env() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        match std::env::var("IRONCALC_RECALC").as_deref() {
            Ok("incremental") => RecalcMode::Incremental,
            Ok("verify") => RecalcMode::Verify,
            _ => RecalcMode::Full,
        }
        #[cfg(target_arch = "wasm32")]
        RecalcMode::Full
    }
}

/// `(sheet, row, column)`.
pub(crate) type Position = (u32, i32, i32);
/// `(sheet, row1, column1, row2, column2)`.
type Area = (u32, i32, i32, i32, i32);

/// Forward dependency edges and the pending dirty set, used to scope an
/// incremental recompute to the cells that can change.
#[derive(Default)]
pub(crate) struct DependencyGraph {
    /// Precedent cell to the cells that reference it.
    cell_dependents: HashMap<Position, Vec<Position>>,
    /// `(range, dependent)` pairs, kept unexpanded so a `SUM` over a large range
    /// does not create an edge per cell.
    range_dependents: Vec<(Area, Position)>,
    dirty: HashSet<Position>,
    /// A value-only edit kept the graph valid, so the next evaluation may run
    /// incrementally.
    incremental_eligible: bool,
    /// A shape-changing edit invalidated the graph, so the next evaluation must
    /// be full. Sticky: a later value edit cannot clear it. Both flags reset
    /// after every evaluation, making full the default.
    forced_full: bool,
    /// Whether a full pass has built the edges. Until it has, there is no graph
    /// to walk.
    graph_built: bool,
}

impl DependencyGraph {
    /// Drops all edges; a full pass rebuilds them.
    pub(crate) fn clear_edges(&mut self) {
        self.cell_dependents.clear();
        self.range_dependents.clear();
    }

    /// Records that `dependent` reads `precedent`.
    pub(crate) fn add_cell_edge(&mut self, precedent: Position, dependent: Position) {
        self.cell_dependents
            .entry(precedent)
            .or_default()
            .push(dependent);
    }

    /// Records that `dependent` reads every cell in `range`.
    pub(crate) fn add_range_edge(&mut self, range: Area, dependent: Position) {
        self.range_dependents.push((range, dependent));
    }

    /// Records a value-only edit, opting the next evaluation into incremental
    /// unless a shape-changing edit has forced a full recompute.
    pub(crate) fn mark_dirty(&mut self, cell: Position) {
        self.dirty.insert(cell);
        if !self.forced_full {
            self.incremental_eligible = true;
        }
    }

    /// Forces the next evaluation to be full and rebuild the graph. Sticky until
    /// that evaluation.
    pub(crate) fn force_full(&mut self) {
        self.forced_full = true;
        self.incremental_eligible = false;
    }

    /// Whether the next evaluation must be full. True unless a value-only edit
    /// opted in, so an un-instrumented mutation is safe.
    pub(crate) fn should_recompute_full(&self) -> bool {
        !self.graph_built || self.forced_full || !self.incremental_eligible
    }

    /// Consumes the dirty set and returns every cell transitively reachable from
    /// it, including the dirty cells.
    pub(crate) fn take_affected(&mut self) -> HashSet<Position> {
        let mut affected = HashSet::new();
        let mut stack: Vec<Position> = self.dirty.drain().collect();
        while let Some(cell) = stack.pop() {
            if !affected.insert(cell) {
                continue;
            }
            if let Some(dependents) = self.cell_dependents.get(&cell) {
                stack.extend(dependents.iter().copied());
            }
            for (range, dependent) in &self.range_dependents {
                if !affected.contains(dependent) && area_contains(range, cell) {
                    stack.push(*dependent);
                }
            }
        }
        affected
    }

    /// Resets pending state after a full pass, which rebuilt the edges.
    pub(crate) fn after_full(&mut self) {
        self.graph_built = true;
        self.clear_pending();
    }

    /// Resets pending state after an incremental pass.
    pub(crate) fn after_incremental(&mut self) {
        self.clear_pending();
    }

    fn clear_pending(&mut self) {
        self.dirty.clear();
        self.incremental_eligible = false;
        self.forced_full = false;
    }
}

fn area_contains(&(sheet, row1, column1, row2, column2): &Area, (s, r, c): Position) -> bool {
    s == sheet && r >= row1 && r <= row2 && c >= column1 && c <= column2
}
