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
    /// Run incremental then full and `assert_eq!` the two agree. Test-only,
    /// gated behind the `recalc_verify` feature so it never ships.
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
/// Rows per band in the per-sheet range membership index. A row-bounded range is
/// registered in each band it spans; a cell scans only the ranges in its band.
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
    dirty: HashSet<Position>,
    /// A value-only edit kept the graph valid, so the next evaluation may run
    /// incrementally.
    incremental_eligible: bool,
    /// A shape-changing edit invalidated the graph, forcing the next pass to be
    /// full. Sticky until then; reset after every pass, so full is the default.
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
    /// the dependents the edit can change, then shifts stored positions to match.
    /// Falls back to full when the shift cannot model the edit.
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

    /// Marks the dependents a structural edit at `boundary` can change: those
    /// reading a moved precedent or a range reaching it. Uses pre-shift
    /// coordinates; [`shift`](Self::shift) then moves the dirty set with the rest.
    fn mark_structural_dependents(&mut self, sheet: u32, axis: Axis, boundary: i32) {
        for (precedent, dependents) in &self.cell_dependents {
            if precedent.0 == sheet && axis.coord(*precedent) >= boundary {
                self.dirty.extend(dependents.iter().copied());
            }
        }
        if let Some(sheet_ranges) = self.range_dependents.get(&sheet) {
            for (area, dependents) in sheet_ranges.iter() {
                if axis.area_max(*area) >= boundary {
                    self.dirty.extend(dependents.iter().copied());
                }
            }
        }
        if !self.forced_full {
            self.incremental_eligible = true;
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

#[cfg(test)]
mod tests {
    use super::*;

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
