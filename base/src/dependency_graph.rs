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
    /// Testing and development only: run incremental, then full, and
    /// `assert_eq!` that they produce identical values, panicking on any
    /// divergence. Not for production use.
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

/// Axis a structural edit inserts or deletes lines along.
#[derive(Clone, Copy)]
pub(crate) enum Axis {
    Row,
    Column,
}

impl Axis {
    fn coord(self, (_, row, column): Position) -> i32 {
        match self {
            Axis::Row => row,
            Axis::Column => column,
        }
    }

    fn area_max(self, (_, _, _, row2, column2): Area) -> i32 {
        match self {
            Axis::Row => row2,
            Axis::Column => column2,
        }
    }

    fn area_min(self, (_, row1, column1, _, _): Area) -> i32 {
        match self {
            Axis::Row => row1,
            Axis::Column => column1,
        }
    }
}

/// New coordinate after inserting (`delta > 0`) or deleting (`delta < 0`)
/// `|delta|` lines at `boundary`. `None` when the line falls inside a deleted
/// band `[boundary, boundary - delta)`.
fn shift_coord(x: i32, boundary: i32, delta: i32) -> Option<i32> {
    if x < boundary {
        Some(x)
    } else if delta < 0 && x < boundary - delta {
        None
    } else {
        Some(x + delta)
    }
}

fn shift_position(
    sheet: u32,
    axis: Axis,
    boundary: i32,
    delta: i32,
    pos: Position,
) -> Option<Position> {
    let (s, row, column) = pos;
    if s != sheet {
        return Some(pos);
    }
    match axis {
        Axis::Row => shift_coord(row, boundary, delta).map(|r| (s, r, column)),
        Axis::Column => shift_coord(column, boundary, delta).map(|c| (s, row, c)),
    }
}

fn shift_area(sheet: u32, axis: Axis, boundary: i32, delta: i32, area: Area) -> Option<Area> {
    let (s, row1, column1, row2, column2) = area;
    if s != sheet {
        return Some(area);
    }
    match axis {
        Axis::Row => Some((
            s,
            shift_coord(row1, boundary, delta)?,
            column1,
            shift_coord(row2, boundary, delta)?,
            column2,
        )),
        Axis::Column => Some((
            s,
            row1,
            shift_coord(column1, boundary, delta)?,
            row2,
            shift_coord(column2, boundary, delta)?,
        )),
    }
}

/// Forward dependency edges and the pending dirty set, used to scope an
/// incremental recompute to the cells that can change.
#[derive(Default)]
pub(crate) struct DependencyGraph {
    /// Precedent cell to the cells that reference it.
    cell_dependents: HashMap<Position, Vec<Position>>,
    /// Distinct range to the cells that read it, kept unexpanded so a `SUM` over
    /// a large range does not create an edge per cell. Deduplicating the range
    /// means the affected-set walk tests each area once no matter how many
    /// formulas share it, rather than once per referencing formula.
    range_dependents: HashMap<Area, Vec<Position>>,
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
        self.range_dependents
            .entry(range)
            .or_default()
            .push(dependent);
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
        let seeds: Vec<Position> = self.dirty.drain().collect();
        self.reachable(seeds)
    }

    /// Every cell transitively reachable from `seeds`, including the seeds. Does
    /// not touch the dirty set, so `Verify` can use it to find the cells a
    /// volatile can taint.
    pub(crate) fn reachable(&self, seeds: Vec<Position>) -> HashSet<Position> {
        let mut affected = HashSet::new();
        let mut stack = seeds;
        while let Some(cell) = stack.pop() {
            if !affected.insert(cell) {
                continue;
            }
            if let Some(dependents) = self.cell_dependents.get(&cell) {
                stack.extend(dependents.iter().copied());
            }
            for (range, dependents) in &self.range_dependents {
                if area_contains(range, cell) {
                    stack.extend(dependents.iter().filter(|d| !affected.contains(d)).copied());
                }
            }
        }
        affected
    }

    /// Applies a row or column insert (`delta > 0`) or delete (`delta < 0`) to
    /// the graph: marks the dependents the edit can change, then shifts every
    /// stored position to match the displacement so later edits still resolve.
    /// Forces a full recompute when the shift cannot model the edit, so the
    /// caller never has to reason about the fallback.
    pub(crate) fn structural_edit(&mut self, sheet: u32, axis: Axis, boundary: i32, delta: i32) {
        // A delete that shrinks a tracked range would need the range clamped;
        // fall back to full rather than model partial-range removal.
        if delta < 0 && self.range_overlaps_band(sheet, axis, boundary, delta) {
            self.force_full();
            return;
        }
        self.mark_structural_dependents(sheet, axis, boundary);
        self.shift(sheet, axis, boundary, delta);
    }

    /// Marks the dependents whose value a structural edit at `boundary` can
    /// change: those reading a moved precedent or a range reaching the boundary.
    /// Uses pre-shift coordinates; [`shift`](Self::shift) then carries the dirty
    /// set along with every other position.
    fn mark_structural_dependents(&mut self, sheet: u32, axis: Axis, boundary: i32) {
        for (precedent, dependents) in &self.cell_dependents {
            if precedent.0 == sheet && axis.coord(*precedent) >= boundary {
                self.dirty.extend(dependents.iter().copied());
            }
        }
        for (area, dependents) in &self.range_dependents {
            if area.0 == sheet && axis.area_max(*area) >= boundary {
                self.dirty.extend(dependents.iter().copied());
            }
        }
        if !self.forced_full {
            self.incremental_eligible = true;
        }
    }

    fn range_overlaps_band(&self, sheet: u32, axis: Axis, boundary: i32, delta: i32) -> bool {
        let band_end = boundary - delta - 1;
        self.range_dependents.keys().any(|area| {
            area.0 == sheet && axis.area_max(*area) >= boundary && axis.area_min(*area) <= band_end
        })
    }

    /// Rewrites every stored position for a displacement at `boundary`. Edges
    /// and ranges landing in a deleted band are dropped; the next full pass
    /// rebuilds them.
    fn shift(&mut self, sheet: u32, axis: Axis, boundary: i32, delta: i32) {
        let shift_pos = |p| shift_position(sheet, axis, boundary, delta, p);
        let mut shifted: HashMap<Position, Vec<Position>> = HashMap::new();
        for (precedent, dependents) in self.cell_dependents.drain() {
            let Some(precedent) = shift_pos(precedent) else {
                continue;
            };
            let dependents: Vec<Position> = dependents.into_iter().filter_map(shift_pos).collect();
            if !dependents.is_empty() {
                shifted.entry(precedent).or_default().extend(dependents);
            }
        }
        self.cell_dependents = shifted;
        let mut shifted_ranges: HashMap<Area, Vec<Position>> = HashMap::new();
        for (area, dependents) in self.range_dependents.drain() {
            let Some(area) = shift_area(sheet, axis, boundary, delta, area) else {
                continue;
            };
            let dependents: Vec<Position> = dependents.into_iter().filter_map(shift_pos).collect();
            if !dependents.is_empty() {
                shifted_ranges.entry(area).or_default().extend(dependents);
            }
        }
        self.range_dependents = shifted_ranges;
        self.dirty = self.dirty.drain().filter_map(shift_pos).collect();
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
