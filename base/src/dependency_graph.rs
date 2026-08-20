//! Dependency graph and recalculation mode for incremental evaluation.
//!
//! A forward dependency graph (precedent to dependents), rebuilt on every full
//! pass from a static walk of the formulas, lets
//! [`Model::evaluate`](crate::Model::evaluate) recompute only the cells reachable
//! from those that changed. Anything the incremental path cannot model forces the
//! next pass to be full, so incremental never diverges from what full produces.

use std::collections::{HashMap, HashSet};

/// Strategy [`Model::evaluate`](crate::Model::evaluate) uses to recompute the
/// workbook, chosen at construction via
/// [`Model::with_recalc_mode`](crate::Model::with_recalc_mode).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RecalcMode {
    /// Recompute every cell. The default and original behavior.
    #[default]
    Full,
    /// Recompute only the cells reachable from the dirty set.
    Incremental,
    /// On Incremental passes, run full as well and `assert_eq!` the two agree.
    /// Formula, first-eval, and array/spill fallbacks are Full and are not
    /// compared. Test-only, gated behind `recalc_verify` so it never ships.
    #[cfg(feature = "recalc_verify")]
    Verify,
}

impl RecalcMode {
    /// The strategy a new model starts in: always `Full` in production. Only test
    /// and `recalc_verify` builds read `IRONCALC_RECALC` (`incremental`/`verify`),
    /// so the suite can run end to end under a chosen strategy.
    pub(crate) fn from_env() -> Self {
        #[cfg(all(not(target_arch = "wasm32"), any(test, feature = "recalc_verify")))]
        {
            match std::env::var("IRONCALC_RECALC").as_deref() {
                Ok("incremental") => RecalcMode::Incremental,
                #[cfg(feature = "recalc_verify")]
                Ok("verify") => RecalcMode::Verify,
                _ => RecalcMode::Full,
            }
        }
        #[cfg(not(all(not(target_arch = "wasm32"), any(test, feature = "recalc_verify"))))]
        {
            RecalcMode::Full
        }
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
    pub(crate) fn coord(self, (_, row, column): Position) -> i32 {
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

pub(crate) fn shift_position(
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

/// Rows per band in the per-sheet range membership index.
const RANGE_INDEX_BAND_ROWS: i32 = 256;
/// A range spanning more than this many bands (a near-full-column reference) is
/// kept in a small always-scanned list instead of being written into every band.
const RANGE_INDEX_MAX_BANDS: i32 = 16;

/// The ranges read on one sheet and their dependents, with a row-band index so a
/// cell can find the ranges that contain it without scanning them all.
/// `dependents` is the source of truth; `bands` and `wide` only speed up the
/// point query and are rebuilt from it whenever positions shift.
#[derive(Default)]
struct SheetRanges {
    dependents: HashMap<Area, HashSet<Position>>,
    /// Row band (`row / RANGE_INDEX_BAND_ROWS`) to the bounded ranges touching it.
    bands: HashMap<i32, Vec<Area>>,
    /// Near-full-column ranges, scanned on every point query.
    wide: Vec<Area>,
}

impl SheetRanges {
    fn band_of(row: i32) -> i32 {
        row.div_euclid(RANGE_INDEX_BAND_ROWS)
    }

    /// Records that `dependent` reads `range`, indexing the range on first sight.
    fn insert(&mut self, range: Area, dependent: Position) {
        let first_sight = !self.dependents.contains_key(&range);
        self.dependents.entry(range).or_default().insert(dependent);
        if !first_sight {
            return;
        }
        let (first_band, last_band) = (Self::band_of(range.1), Self::band_of(range.3));
        if last_band - first_band >= RANGE_INDEX_MAX_BANDS {
            self.wide.push(range);
        } else {
            for band in first_band..=last_band {
                self.bands.entry(band).or_default().push(range);
            }
        }
    }

    /// The ranges on this sheet that contain `cell`.
    fn containing(&self, cell: Position) -> impl Iterator<Item = &Area> + '_ {
        self.wide
            .iter()
            .chain(self.bands.get(&Self::band_of(cell.1)).into_iter().flatten())
            .filter(move |&range| area_contains(range, cell))
    }

    fn dependents_of(&self, range: &Area) -> Option<&HashSet<Position>> {
        self.dependents.get(range)
    }

    fn iter(&self) -> impl Iterator<Item = (&Area, &HashSet<Position>)> + '_ {
        self.dependents.iter()
    }

    fn areas(&self) -> impl Iterator<Item = &Area> + '_ {
        self.dependents.keys()
    }
}

/// Walkability of the stored edges. One enum so built/forced flags cannot disagree.
#[derive(Default)]
enum GraphState {
    Ready {
        dirty: HashSet<Position>,
    },
    #[default]
    MustRebuild,
}

/// Shared storage for the role-typed sets on [`DependencyGraph`].
#[derive(Default)]
struct Positions(HashSet<Position>);

impl Positions {
    fn contains(&self, cell: &Position) -> bool {
        self.0.contains(cell)
    }

    fn iter(&self) -> impl Iterator<Item = Position> + '_ {
        self.0.iter().copied()
    }

    fn replace(&mut self, cells: HashSet<Position>) {
        self.0 = cells;
    }

    fn shift(&mut self, shift_pos: impl Fn(Position) -> Option<Position>) {
        self.0 = self.0.drain().filter_map(shift_pos).collect();
    }
}

/// Array/spill cells.
#[derive(Default)]
pub(crate) struct ArrayCells(Positions);

/// Cells that re-roll every pass (`RAND`, `NOW`, …).
#[derive(Default)]
pub(crate) struct VolatileCells(Positions);

/// RAND/NOW/TODAY. Verify strips this cone.
#[derive(Default)]
pub(crate) struct NondeterministicCells(Positions);

impl ArrayCells {
    pub(crate) fn contains(&self, cell: &Position) -> bool {
        self.0.contains(cell)
    }

    fn replace(&mut self, cells: HashSet<Position>) {
        self.0.replace(cells);
    }

    fn shift(&mut self, shift_pos: impl Fn(Position) -> Option<Position>) {
        self.0.shift(shift_pos);
    }
}

impl VolatileCells {
    #[cfg(test)]
    pub(crate) fn contains(&self, cell: &Position) -> bool {
        self.0.contains(cell)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = Position> + '_ {
        self.0.iter()
    }

    fn replace(&mut self, cells: HashSet<Position>) {
        self.0.replace(cells);
    }

    fn shift(&mut self, shift_pos: impl Fn(Position) -> Option<Position>) {
        self.0.shift(shift_pos);
    }
}

impl NondeterministicCells {
    pub(crate) fn iter(&self) -> impl Iterator<Item = Position> + '_ {
        self.0.iter()
    }

    fn replace(&mut self, cells: HashSet<Position>) {
        self.0.replace(cells);
    }

    fn shift(&mut self, shift_pos: impl Fn(Position) -> Option<Position>) {
        self.0.shift(shift_pos);
    }
}

#[derive(Default)]
pub(crate) struct DependencyGraph {
    /// Precedent cell to the cells that reference it. A set, so a formula reading
    /// the same cell twice (`=A1+A1`) records a single edge.
    cell_dependents: HashMap<Position, HashSet<Position>>,
    /// Per sheet, the ranges read on it and their dependents. Ranges are kept
    /// unexpanded so a `SUM` over a large range costs one edge, not one per cell,
    /// and each fires once per walk however many formulas share it. Bucketed by
    /// sheet (like HyperFormula's range mapping) and indexed by row band within a
    /// sheet, so a cell finds the ranges containing it without scanning them all.
    range_dependents: HashMap<u32, SheetRanges>,
    state: GraphState,
    pub(crate) volatile: VolatileCells,
    pub(crate) arrays: ArrayCells,
    pub(crate) nondeterministic: NondeterministicCells,
}

impl DependencyGraph {
    /// Drops all edges; a full pass rebuilds them.
    pub(crate) fn clear_edges(&mut self) {
        self.cell_dependents.clear();
        self.range_dependents.clear();
    }

    /// Records that `dependent` reads `precedent`. Idempotent.
    pub(crate) fn add_cell_edge(&mut self, precedent: Position, dependent: Position) {
        self.cell_dependents
            .entry(precedent)
            .or_default()
            .insert(dependent);
    }

    /// Records that `dependent` reads every cell in `range`. Idempotent.
    pub(crate) fn add_range_edge(&mut self, range: Area, dependent: Position) {
        self.range_dependents
            .entry(range.0)
            .or_default()
            .insert(range, dependent);
    }

    /// Records a value-only edit. Only a [`GraphState::Ready`] graph can opt into
    /// incremental; `MustRebuild` stays full.
    pub(crate) fn mark_dirty(&mut self, cell: Position) {
        if let GraphState::Ready { dirty } = &mut self.state {
            dirty.insert(cell);
        }
    }

    /// Forces the next evaluation to be full and rebuild the graph.
    pub(crate) fn force_full(&mut self) {
        self.state = GraphState::MustRebuild;
    }

    /// Whether the next evaluation must be full. True unless the graph is ready
    /// and something is dirty, so an un-instrumented mutation is safe.
    pub(crate) fn should_recompute_full(&self) -> bool {
        !matches!(&self.state, GraphState::Ready { dirty } if !dirty.is_empty())
    }

    /// True when the graph is not ready: first pass or an unmodeled edit.
    #[cfg(test)]
    pub(crate) fn full_reflects_change(&self) -> bool {
        !matches!(self.state, GraphState::Ready { .. })
    }

    /// Dirty cells and the cells reachable from them.
    pub(crate) fn take_seeds_and_affected(&mut self) -> (Vec<Position>, HashSet<Position>) {
        let GraphState::Ready { dirty } = &mut self.state else {
            return (Vec::new(), HashSet::new());
        };
        let seeds: Vec<Position> = std::mem::take(dirty).into_iter().collect();
        let affected = self.reachable(seeds.clone());
        (seeds, affected)
    }

    pub(crate) fn replace_volatile(&mut self, cells: HashSet<Position>) {
        self.volatile.replace(cells);
    }

    pub(crate) fn replace_arrays(&mut self, cells: HashSet<Position>) {
        self.arrays.replace(cells);
    }

    pub(crate) fn replace_nondeterministic(&mut self, cells: HashSet<Position>) {
        self.nondeterministic.replace(cells);
    }

    /// Every cell transitively reachable from `seeds`, including the seeds. Does
    /// not touch the dirty set, so `Verify` can use it to find the cells a
    /// volatile can taint.
    pub(crate) fn reachable(&self, seeds: Vec<Position>) -> HashSet<Position> {
        let mut affected = HashSet::new();
        let mut stack = seeds;
        // A range's dependents all become affected the moment any one of its
        // cells does, so fire each range at most once and drop it from the scan.
        let mut fired: HashSet<Area> = HashSet::new();
        while let Some(cell) = stack.pop() {
            if !affected.insert(cell) {
                continue;
            }
            if let Some(dependents) = self.cell_dependents.get(&cell) {
                stack.extend(dependents.iter().copied());
            }
            if let Some(sheet_ranges) = self.range_dependents.get(&cell.0) {
                for range in sheet_ranges.containing(cell) {
                    if !fired.insert(*range) {
                        continue;
                    }
                    if let Some(dependents) = sheet_ranges.dependents_of(range) {
                        stack.extend(dependents.iter().filter(|d| !affected.contains(d)).copied());
                    }
                }
            }
        }
        affected
    }

    /// Applies a row/column insert (`delta > 0`) or delete (`delta < 0`): marks
    /// the dependents the edit can change, then shifts every stored position.
    pub(crate) fn structural_edit(&mut self, sheet: u32, axis: Axis, boundary: i32, delta: i32) {
        // A delete that shrinks a tracked range would need the range clamped.
        if delta < 0 && self.range_overlaps_band(sheet, axis, boundary, delta) {
            self.state = GraphState::MustRebuild;
            return;
        }
        if !matches!(self.state, GraphState::Ready { .. }) {
            self.state = GraphState::MustRebuild;
            return;
        }
        self.mark_structural_dependents(sheet, axis, boundary);
        self.shift(sheet, axis, boundary, delta);
    }

    /// Marks the dependents a structural edit at `boundary` can change: those
    /// reading a moved precedent or a range reaching it. Uses pre-shift
    /// coordinates; [`shift`](Self::shift) then moves the dirty set with the rest.
    fn mark_structural_dependents(&mut self, sheet: u32, axis: Axis, boundary: i32) {
        let GraphState::Ready { dirty } = &mut self.state else {
            return;
        };
        for (precedent, dependents) in &self.cell_dependents {
            if precedent.0 == sheet && axis.coord(*precedent) >= boundary {
                dirty.extend(dependents.iter().copied());
            }
        }
        if let Some(sheet_ranges) = self.range_dependents.get(&sheet) {
            for (area, dependents) in sheet_ranges.iter() {
                if axis.area_max(*area) >= boundary {
                    dirty.extend(dependents.iter().copied());
                }
            }
        }
    }

    fn range_overlaps_band(&self, sheet: u32, axis: Axis, boundary: i32, delta: i32) -> bool {
        let band_end = boundary - delta - 1;
        self.range_dependents
            .get(&sheet)
            .is_some_and(|sheet_ranges| {
                sheet_ranges.areas().any(|area| {
                    axis.area_max(*area) >= boundary && axis.area_min(*area) <= band_end
                })
            })
    }

    /// Rewrites every stored position for a displacement at `boundary`. Edges
    /// and ranges landing in a deleted band are dropped; the next full pass
    /// rebuilds them.
    fn shift(&mut self, sheet: u32, axis: Axis, boundary: i32, delta: i32) {
        let shift_pos = |p| shift_position(sheet, axis, boundary, delta, p);
        let mut shifted: HashMap<Position, HashSet<Position>> = HashMap::new();
        for (precedent, dependents) in self.cell_dependents.drain() {
            let Some(precedent) = shift_pos(precedent) else {
                continue;
            };
            let dependents: HashSet<Position> =
                dependents.into_iter().filter_map(shift_pos).collect();
            if !dependents.is_empty() {
                shifted.entry(precedent).or_default().extend(dependents);
            }
        }
        self.cell_dependents = shifted;
        let mut shifted_ranges: HashMap<u32, SheetRanges> = HashMap::new();
        for (area, dependents) in self
            .range_dependents
            .drain()
            .flat_map(|(_, sheet_ranges)| sheet_ranges.dependents)
        {
            let Some(area) = shift_area(sheet, axis, boundary, delta, area) else {
                continue;
            };
            for dependent in dependents.into_iter().filter_map(shift_pos) {
                shifted_ranges
                    .entry(area.0)
                    .or_default()
                    .insert(area, dependent);
            }
        }
        self.range_dependents = shifted_ranges;
        if let GraphState::Ready { dirty } = &mut self.state {
            *dirty = dirty.drain().filter_map(shift_pos).collect();
        }
        self.volatile.shift(shift_pos);
        self.arrays.shift(shift_pos);
        self.nondeterministic.shift(shift_pos);
    }

    /// Marks the graph ready after a pass that left the edges valid.
    pub(crate) fn after_pass(&mut self) {
        self.state = GraphState::Ready {
            dirty: HashSet::new(),
        };
    }
}

fn area_contains(&(sheet, row1, column1, row2, column2): &Area, (s, r, c): Position) -> bool {
    s == sheet && r >= row1 && r <= row2 && c >= column1 && c <= column2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_state_is_explicit() {
        let mut graph = DependencyGraph::default();
        assert!(graph.full_reflects_change());
        assert!(graph.should_recompute_full());

        graph.after_pass();
        assert!(!graph.full_reflects_change());
        assert!(graph.should_recompute_full()); // Ready, nothing dirty

        graph.mark_dirty((0, 1, 1));
        assert!(!graph.should_recompute_full());
        let (_seeds, affected) = graph.take_seeds_and_affected();
        assert!(affected.contains(&(0, 1, 1)));
        assert!(graph.should_recompute_full()); // dirty consumed

        graph.force_full();
        assert!(graph.full_reflects_change());
        assert!(graph.should_recompute_full());
        graph.mark_dirty((0, 2, 1)); // ignored: not Ready
        assert!(graph.should_recompute_full());
    }

    #[test]
    fn range_index_matches_brute_force() {
        let ranges: [Area; 5] = [
            (0, 1, 1, 5, 5),
            (0, 1, 1, 1000, 1),
            (0, 1, 1, 2_000_000, 3), // near-full-column, kept in the wide list
            (0, 300, 2, 320, 4),
            (0, 257, 1, 258, 1), // straddles a band boundary
        ];
        let mut sheet_ranges = SheetRanges::default();
        for (i, range) in ranges.iter().enumerate() {
            sheet_ranges.insert(*range, (0, i as i32, 0));
        }
        for row in [1, 5, 6, 256, 257, 258, 300, 320, 1000, 1001, 1_500_000] {
            for column in [1, 2, 3, 4, 5, 6] {
                let cell = (0, row, column);
                let mut got: Vec<Area> = sheet_ranges.containing(cell).copied().collect();
                got.sort_unstable();
                let mut want: Vec<Area> = ranges
                    .iter()
                    .copied()
                    .filter(|range| area_contains(range, cell))
                    .collect();
                want.sort_unstable();
                assert_eq!(got, want, "cell {cell:?}");
            }
        }
    }
}
