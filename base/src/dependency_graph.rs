//! Dependency graph and recalculation mode for incremental evaluation.
//!
//! A forward dependency graph (precedent to dependents), rebuilt from the
//! reads observed while formulas evaluate, lets
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
    ///
    /// Multi-column `SUM` may re-associate by composing per-row subtotals, so
    /// floating-point order can differ from default Full's left-to-right
    /// row-major scan. That difference is intentional and isolated to this
    /// mode and Verify; default Full stays a single accumulator.
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
pub(crate) type Area = (u32, i32, i32, i32, i32);

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
#[derive(Clone, Default)]
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

    /// Drops `dependent` from this range, pruning the `bands`/`wide` index
    /// entry when the range loses its last dependent. Without the prune, a
    /// re-read on the next pass would treat the range as "first sight" again
    /// and append duplicate band entries, so `containing()` scans the same
    /// area once per historical pass — unbounded growth over a model's life.
    fn remove_dependent(&mut self, range: &Area, dependent: &Position) {
        if let Some(set) = self.dependents.get_mut(range) {
            set.remove(dependent);
            if set.is_empty() {
                self.dependents.remove(range);
                self.deindex(range);
            }
        }
    }

    /// Removes one occurrence of `range` from the point-query index.
    fn deindex(&mut self, range: &Area) {
        let (first_band, last_band) = (Self::band_of(range.1), Self::band_of(range.3));
        if last_band - first_band >= RANGE_INDEX_MAX_BANDS {
            if let Some(pos) = self.wide.iter().position(|a| a == range) {
                self.wide.swap_remove(pos);
            }
            return;
        }
        for band in first_band..=last_band {
            if let Some(list) = self.bands.get_mut(&band) {
                if let Some(pos) = list.iter().position(|a| a == range) {
                    list.swap_remove(pos);
                }
                if list.is_empty() {
                    self.bands.remove(&band);
                }
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
#[derive(Clone, Default)]
enum GraphState {
    Ready {
        dirty: HashSet<Position>,
    },
    #[default]
    MustRebuild,
}

/// Shared storage for the role-typed sets on [`DependencyGraph`].
#[derive(Clone, Default)]
struct Positions(HashSet<Position>);

impl Positions {
    fn contains(&self, cell: &Position) -> bool {
        self.0.contains(cell)
    }

    fn replace(&mut self, cells: HashSet<Position>) {
        self.0 = cells;
    }

    fn shift(&mut self, shift_pos: impl Fn(Position) -> Option<Position>) {
        self.0 = self.0.drain().filter_map(shift_pos).collect();
    }
}

/// Array/spill cells.
#[derive(Clone, Default)]
pub(crate) struct ArrayCells(Positions);

impl ArrayCells {
    pub(crate) fn contains(&self, cell: &Position) -> bool {
        self.0.contains(cell)
    }

    pub(crate) fn snapshot(&self) -> HashSet<Position> {
        self.0 .0.clone()
    }

    fn replace(&mut self, cells: HashSet<Position>) {
        self.0.replace(cells);
    }

    fn shift(&mut self, shift_pos: impl Fn(Position) -> Option<Position>) {
        self.0.shift(shift_pos);
    }
}

#[derive(Clone, Default)]
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
    /// Non-cell inputs (hidden flags, own coordinates, clock, names…) to the
    /// formulas that read them.
    input_dependents: HashMap<crate::recalc::Input, HashSet<Position>>,
    /// What each formula read last time it evaluated; the reverse of the edge
    /// maps, so a formula's edges can be dropped in O(degree) before re-record.
    precedents: HashMap<Position, crate::recalc::ReadSet>,
    state: GraphState,
    pub(crate) arrays: ArrayCells,
    /// Insert/delete can move data cells the dirty cone does not name. Cleared
    /// in [`Self::after_pass`] so it cannot leak across a Full fallback.
    structural_unknown: bool,
}

impl DependencyGraph {
    /// Drops all edges; a full pass rebuilds them.
    pub(crate) fn clear_edges(&mut self) {
        self.cell_dependents.clear();
        self.range_dependents.clear();
        self.input_dependents.clear();
        self.precedents.clear();
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

    /// Records that `dependent` reads a non-cell input. Idempotent.
    pub(crate) fn add_input_edge(&mut self, input: crate::recalc::Input, dependent: Position) {
        self.input_dependents
            .entry(input)
            .or_default()
            .insert(dependent);
    }

    /// Formulas that read any input for which `pred` holds.
    pub(crate) fn dependents_of_inputs(
        &self,
        pred: impl Fn(&crate::recalc::Input) -> bool,
    ) -> HashSet<Position> {
        self.input_dependents
            .iter()
            .filter(|(input, _)| pred(input))
            .flat_map(|(_, deps)| deps.iter().copied())
            .collect()
    }

    /// RAND/NOW/TODAY and CELL/INFO: re-roll every pass and always-report.
    pub(crate) fn always_dirty_cells(&self) -> HashSet<Position> {
        self.dependents_of_inputs(|i| {
            matches!(
                i,
                crate::recalc::Input::Random
                    | crate::recalc::Input::Clock
                    | crate::recalc::Input::Environment
            )
        })
    }

    /// Replaces `dependent`'s outgoing edges with the reads just observed.
    pub(crate) fn replace_reads(&mut self, dependent: Position, reads: crate::recalc::ReadSet) {
        self.remove_dependent(dependent);
        for &p in &reads.cells {
            if p != dependent {
                self.add_cell_edge(p, dependent);
            }
        }
        for &area in &reads.rects {
            self.add_range_edge(area, dependent);
        }
        for input in &reads.inputs {
            self.add_input_edge(input.clone(), dependent);
        }
        self.precedents.insert(dependent, reads);
    }

    /// Drops outgoing edges from a cell in O(degree) via the reverse index.
    pub(crate) fn remove_dependent(&mut self, dependent: Position) {
        let Some(reads) = self.precedents.remove(&dependent) else {
            return;
        };
        for p in reads.cells {
            if let Some(set) = self.cell_dependents.get_mut(&p) {
                set.remove(&dependent);
                if set.is_empty() {
                    self.cell_dependents.remove(&p);
                }
            }
        }
        for area in reads.rects {
            if let Some(sheet) = self.range_dependents.get_mut(&area.0) {
                sheet.remove_dependent(&area, &dependent);
            }
        }
        for input in reads.inputs {
            if let Some(set) = self.input_dependents.get_mut(&input) {
                set.remove(&dependent);
                if set.is_empty() {
                    self.input_dependents.remove(&input);
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn cell_reads(
        &self,
        cell: Position,
        pred: impl Fn(&crate::recalc::Input) -> bool,
    ) -> bool {
        self.precedents
            .get(&cell)
            .is_some_and(|reads| reads.inputs.iter().any(pred))
    }

    /// Records a value-only edit. Only a [`GraphState::Ready`] graph can opt into
    /// incremental; `MustRebuild` stays full.
    pub(crate) fn mark_dirty(&mut self, cell: Position) {
        if let GraphState::Ready { dirty } = &mut self.state {
            dirty.insert(cell);
        }
    }

    /// Seeds Verify allows in the delta even when the snapshot did not move:
    /// user edits plus RAND/NOW/TODAY/CELL. OFFSET recomputes because its
    /// actual target is a traced edge; it is not always-report.
    #[cfg(feature = "recalc_verify")]
    pub(crate) fn always_report_seeds(&self) -> HashSet<Position> {
        let mut seeds = match &self.state {
            GraphState::Ready { dirty } => dirty.clone(),
            GraphState::MustRebuild => HashSet::new(),
        };
        seeds.extend(self.always_dirty_cells());
        seeds
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

    pub(crate) fn replace_arrays(&mut self, cells: HashSet<Position>) {
        self.arrays.replace(cells);
    }

    /// Direct dependents of `cell`: cells reading it, cells reading a range
    /// that contains it, and cells reading a name that currently resolves to it.
    pub(crate) fn dependents_of(&self, cell: Position) -> Vec<Position> {
        let mut dependents: Vec<Position> = self
            .cell_dependents
            .get(&cell)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        if let Some(sheet_ranges) = self.range_dependents.get(&cell.0) {
            for range in sheet_ranges.containing(cell) {
                if let Some(range_dependents) = sheet_ranges.dependents_of(range) {
                    dependents.extend(range_dependents.iter().copied());
                }
            }
        }
        dependents
    }

    /// Orders `affected` so each cell follows the affected cells it reads.
    /// Returns `None` when the affected set contains a dependency cycle, so the
    /// caller can fall back to the recursive recompute that reports `#CIRC!`.
    pub(crate) fn topo_order(&self, affected: &HashSet<Position>) -> Option<Vec<Position>> {
        let successors = |cell: Position| -> Vec<Position> {
            let mut dependents: Vec<Position> = self
                .dependents_of(cell)
                .into_iter()
                .filter(|d| affected.contains(d))
                .collect();
            dependents.sort_unstable();
            dependents
        };
        let mut indegree: HashMap<Position, usize> = affected.iter().map(|&c| (c, 0)).collect();
        for &cell in affected {
            for dependent in successors(cell) {
                *indegree.entry(dependent).or_default() += 1;
            }
        }
        let mut queue: Vec<Position> = indegree
            .iter()
            .filter(|(_, &n)| n == 0)
            .map(|(&c, _)| c)
            .collect();
        queue.sort_unstable();
        let mut order = Vec::with_capacity(affected.len());
        let mut head = 0;
        while head < queue.len() {
            let cell = queue[head];
            head += 1;
            order.push(cell);
            for dependent in successors(cell) {
                let n = indegree.entry(dependent).or_default();
                *n -= 1;
                if *n == 0 {
                    queue.push(dependent);
                }
            }
        }
        (order.len() == affected.len()).then_some(order)
    }

    /// Every cell transitively reachable from `seeds`, including the seeds. Does
    /// not touch the dirty set. Verify uses it on the RAND/NOW/TODAY cone only.
    /// `OFFSET` is not stripped (compared when Incremental). A top-level
    /// `INDIRECT` is a 1×1 dynamic array (Full, not compared). Wrapped
    /// `INDIRECT` (`SUM`/`PRODUCT`) stays Incremental.
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
        // Data cells in the shift band are not dirty. The cell-list delta cannot
        // name them, so `take_changed_cells` reports Everything after this pass.
        self.structural_unknown = true;
    }

    /// Marks the dependents a structural edit at `boundary` can change: those
    /// reading a moved precedent or a range reaching it. Uses pre-shift
    /// coordinates; [`shift`](Self::shift) then moves the dirty set with the rest.
    fn mark_structural_dependents(&mut self, sheet: u32, axis: Axis, boundary: i32) {
        if !matches!(self.state, GraphState::Ready { .. }) {
            return;
        }
        let mut extra: HashSet<Position> = HashSet::new();
        for (precedent, dependents) in &self.cell_dependents {
            if precedent.0 == sheet && axis.coord(*precedent) >= boundary {
                extra.extend(dependents.iter().copied());
            }
        }
        if let Some(sheet_ranges) = self.range_dependents.get(&sheet) {
            for (area, dependents) in sheet_ranges.iter() {
                if axis.area_max(*area) >= boundary {
                    extra.extend(dependents.iter().copied());
                }
            }
        }
        // Coordinates, formula text, and name resolutions can change without
        // any precedent value moving. Re-run everything that read them.
        extra.extend(self.dependents_of_inputs(|input| match input {
            crate::recalc::Input::OwnCoord((s, ..))
            | crate::recalc::Input::FormulaText((s, ..)) => *s == sheet,
            crate::recalc::Input::Name { .. }
            | crate::recalc::Input::SheetStructure
            | crate::recalc::Input::Computed => true,
            _ => false,
        }));
        if let GraphState::Ready { dirty } = &mut self.state {
            dirty.extend(extra);
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
        self.arrays.shift(shift_pos);
        let shift_input = |input: crate::recalc::Input| -> Option<crate::recalc::Input> {
            match input {
                crate::recalc::Input::OwnCoord(p) => {
                    Some(crate::recalc::Input::OwnCoord(shift_pos(p)?))
                }
                crate::recalc::Input::FormulaText(p) => {
                    Some(crate::recalc::Input::FormulaText(shift_pos(p)?))
                }
                crate::recalc::Input::RowHidden(s, r)
                    if s == sheet && matches!(axis, Axis::Row) =>
                {
                    Some(crate::recalc::Input::RowHidden(
                        s,
                        shift_coord(r, boundary, delta)?,
                    ))
                }
                crate::recalc::Input::ColHidden(s, c)
                    if s == sheet && matches!(axis, Axis::Column) =>
                {
                    Some(crate::recalc::Input::ColHidden(
                        s,
                        shift_coord(c, boundary, delta)?,
                    ))
                }
                other => Some(other),
            }
        };
        let shifted_inputs: HashMap<crate::recalc::Input, HashSet<Position>> = self
            .input_dependents
            .drain()
            .filter_map(|(input, deps)| {
                let deps: HashSet<Position> = deps.into_iter().filter_map(shift_pos).collect();
                if deps.is_empty() {
                    return None;
                }
                Some((shift_input(input)?, deps))
            })
            .collect();
        self.input_dependents = shifted_inputs;
        self.precedents = self
            .precedents
            .drain()
            .filter_map(|(dep, reads)| {
                let dep = shift_pos(dep)?;
                Some((
                    dep,
                    crate::recalc::ReadSet {
                        cells: reads.cells.into_iter().filter_map(shift_pos).collect(),
                        rects: reads
                            .rects
                            .into_iter()
                            .filter_map(|area| shift_area(sheet, axis, boundary, delta, area))
                            .collect(),
                        inputs: reads.inputs.into_iter().filter_map(shift_input).collect(),
                    },
                ))
            })
            .collect();
    }

    /// Whether this pass follows an insert/delete whose delta cannot name every
    /// moved cell. Clears the flag.
    pub(crate) fn take_structural_unknown(&mut self) -> bool {
        std::mem::replace(&mut self.structural_unknown, false)
    }

    /// Marks the graph ready after a pass that left the edges valid.
    pub(crate) fn after_pass(&mut self) {
        self.structural_unknown = false;
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

    #[test]
    fn removing_last_dependent_prunes_the_index() {
        let mut sheet_ranges = SheetRanges::default();
        let range: Area = (0, 1, 1, 300, 1);
        let dependent: Position = (0, 1, 10);
        sheet_ranges.insert(range, dependent);
        sheet_ranges.remove_dependent(&range, &dependent);
        // The stale entry must be gone: re-inserting must not duplicate.
        sheet_ranges.insert(range, dependent);
        let count = sheet_ranges
            .containing((0, 150, 1))
            .filter(|a| *a == &range)
            .count();
        assert_eq!(count, 1, "duplicate band entries after re-insert");

        let wide: Area = (0, 1, 1, 2_000_000, 3);
        sheet_ranges.insert(wide, (0, 2, 10));
        sheet_ranges.remove_dependent(&wide, &(0, 2, 10));
        sheet_ranges.insert(wide, (0, 3, 10));
        let count = sheet_ranges
            .containing((0, 500_000, 3))
            .filter(|a| *a == &wide)
            .count();
        assert_eq!(count, 1, "duplicate wide entries after re-insert");
    }
}
