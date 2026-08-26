//! Dependency graph and recalculation mode for incremental evaluation.
//!
//! A forward dependency graph (precedent to dependents), rebuilt from the
//! reads observed while formulas evaluate, lets
//! [`Model::evaluate`](crate::Model::evaluate) recompute only the cells reachable
//! from those that changed. Anything the incremental path cannot model forces the
//! next pass to be full, so incremental never diverges from what full produces.

use std::collections::{HashMap, HashSet};

use crate::recalc::{Input, ReadSet};

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
    /// The coordinate of `position` along this axis: the one an edit on this
    /// axis moves, and the one to compare against a boundary.
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

/// One row/column insert (`delta > 0`) or delete (`delta < 0`) of `|delta|`
/// lines at `boundary` on `sheet`, seen as a coordinate remapping.
///
/// Every coordinate the graph stores is rewritten through this one value, so
/// the remapping rule has a single definition. `None` from any of its methods
/// means the coordinate fell inside a deleted band: the entry holding it is
/// dropped, and the next full pass rebuilds it.
#[derive(Clone, Copy)]
pub(crate) struct Displacement {
    sheet: u32,
    axis: Axis,
    boundary: i32,
    delta: i32,
}

impl Displacement {
    fn coord(self, x: i32) -> Option<i32> {
        shift_coord(x, self.boundary, self.delta)
    }

    /// The new location of `pos`, or `None` if it was deleted.
    fn position(self, pos: Position) -> Option<Position> {
        let (s, row, column) = pos;
        if s != self.sheet {
            return Some(pos);
        }
        match self.axis {
            Axis::Row => self.coord(row).map(|r| (s, r, column)),
            Axis::Column => self.coord(column).map(|c| (s, row, c)),
        }
    }

    /// The new extent of `area`, or `None` if either edge was deleted.
    /// A delete that would only *shrink* a tracked range never reaches here:
    /// [`DependencyGraph::structural_edit`] rebuilds instead.
    fn area(self, area: Area) -> Option<Area> {
        let (s, row1, column1, row2, column2) = area;
        if s != self.sheet {
            return Some(area);
        }
        match self.axis {
            Axis::Row => Some((s, self.coord(row1)?, column1, self.coord(row2)?, column2)),
            Axis::Column => Some((s, row1, self.coord(column1)?, row2, self.coord(column2)?)),
        }
    }

    /// The new form of a non-cell input. Only the position- and line-keyed
    /// variants move; the rest are keyed by nothing this edit touches.
    fn input(self, input: Input) -> Option<Input> {
        match input {
            Input::OwnCoord(p) => Some(Input::OwnCoord(self.position(p)?)),
            Input::FormulaText(p) => Some(Input::FormulaText(self.position(p)?)),
            Input::RowHidden(s, r) if s == self.sheet && matches!(self.axis, Axis::Row) => {
                Some(Input::RowHidden(s, self.coord(r)?))
            }
            Input::ColHidden(s, c) if s == self.sheet && matches!(self.axis, Axis::Column) => {
                Some(Input::ColHidden(s, self.coord(c)?))
            }
            other => Some(other),
        }
    }
}

/// The new location of `pos` after an insert (`delta > 0`) or delete
/// (`delta < 0`) of `|delta|` lines at `boundary` on `sheet`, or `None` if the
/// edit deleted it. Positions on other sheets are returned unchanged.
///
/// The same rule the graph shifts itself by; callers outside the graph use it
/// to move positions they hold.
pub(crate) fn shift_position(
    sheet: u32,
    axis: Axis,
    boundary: i32,
    delta: i32,
    pos: Position,
) -> Option<Position> {
    Displacement {
        sheet,
        axis,
        boundary,
        delta,
    }
    .position(pos)
}

/// A stored index whose coordinates move with a structural edit.
///
/// Every positional field of [`DependencyGraph`] implements this, and
/// [`DependencyGraph::shift`] applies it to each one by name. Adding an index
/// and forgetting to shift it has already been a bug here (the banded range
/// index shifted its areas but not its bands); the destructuring in `shift`
/// turns the next occurrence into a compile error.
trait Shift {
    /// Rewrites every coordinate this index stores, dropping entries whose
    /// coordinates were deleted.
    fn shift(&mut self, displacement: Displacement);
}

impl Shift for HashMap<Position, HashSet<Position>> {
    fn shift(&mut self, displacement: Displacement) {
        let mut shifted: HashMap<Position, HashSet<Position>> = HashMap::new();
        for (precedent, dependents) in self.drain() {
            let Some(precedent) = displacement.position(precedent) else {
                continue;
            };
            let dependents: HashSet<Position> = dependents
                .into_iter()
                .filter_map(|p| displacement.position(p))
                .collect();
            if !dependents.is_empty() {
                shifted.entry(precedent).or_default().extend(dependents);
            }
        }
        *self = shifted;
    }
}

impl Shift for HashMap<u32, SheetRanges> {
    fn shift(&mut self, displacement: Displacement) {
        // Reinserting rebuilds `bands`/`wide` from the shifted areas, which is
        // why those two need no shift of their own.
        let mut shifted: HashMap<u32, SheetRanges> = HashMap::new();
        for (area, dependents) in self
            .drain()
            .flat_map(|(_, sheet_ranges)| sheet_ranges.dependents)
        {
            let Some(area) = displacement.area(area) else {
                continue;
            };
            for dependent in dependents
                .into_iter()
                .filter_map(|p| displacement.position(p))
            {
                shifted.entry(area.0).or_default().insert(area, dependent);
            }
        }
        *self = shifted;
    }
}

impl Shift for HashMap<Input, HashSet<Position>> {
    fn shift(&mut self, displacement: Displacement) {
        *self = self
            .drain()
            .filter_map(|(input, deps)| {
                let deps: HashSet<Position> = deps
                    .into_iter()
                    .filter_map(|p| displacement.position(p))
                    .collect();
                if deps.is_empty() {
                    return None;
                }
                Some((displacement.input(input)?, deps))
            })
            .collect();
    }
}

impl Shift for HashMap<Position, ReadSet> {
    fn shift(&mut self, displacement: Displacement) {
        *self = self
            .drain()
            .filter_map(|(dependent, reads)| {
                let dependent = displacement.position(dependent)?;
                Some((
                    dependent,
                    ReadSet {
                        cells: reads
                            .cells
                            .into_iter()
                            .filter_map(|p| displacement.position(p))
                            .collect(),
                        rects: reads
                            .rects
                            .into_iter()
                            .filter_map(|area| displacement.area(area))
                            .collect(),
                        inputs: reads
                            .inputs
                            .into_iter()
                            .filter_map(|input| displacement.input(input))
                            .collect(),
                    },
                ))
            })
            .collect();
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

impl Shift for GraphState {
    fn shift(&mut self, displacement: Displacement) {
        if let GraphState::Ready { dirty } = self {
            *dirty = dirty
                .drain()
                .filter_map(|p| displacement.position(p))
                .collect();
        }
    }
}

/// Shared storage for the role-typed sets on [`DependencyGraph`].
#[derive(Clone, Default)]
struct Positions(HashSet<Position>);

impl Positions {
    fn replace(&mut self, cells: HashSet<Position>) {
        self.0 = cells;
    }
}

impl Shift for Positions {
    fn shift(&mut self, displacement: Displacement) {
        self.0 = self
            .0
            .drain()
            .filter_map(|p| displacement.position(p))
            .collect();
    }
}

/// Array/spill cells, each mapped to the anchor that produces it. The anchor
/// maps to itself. Reading any of these positions is a read of that anchor,
/// which is the only way the graph learns that a formula depends on an array's
/// output: the anchor's writes into its footprint are evaluation writes, not
/// edits, so nothing else records them.
#[derive(Clone, Default)]
pub(crate) struct ArrayCells(HashMap<Position, Position>);

impl ArrayCells {
    /// Whether `cell` lies in some anchor's footprint. An edit that reaches one
    /// of these sends the pass to Full, which is the only pass that spills.
    pub(crate) fn contains(&self, cell: &Position) -> bool {
        self.0.contains_key(cell)
    }

    /// Whether the workbook has no array or spill cells at all.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The anchor whose footprint covers `cell`, if any.
    pub(crate) fn anchor_of(&self, cell: Position) -> Option<Position> {
        self.0.get(&cell).copied()
    }

    /// Every indexed footprint position, anchors included, as an owned set:
    /// for callers that need to walk the index while mutating the model.
    pub(crate) fn snapshot(&self) -> HashSet<Position> {
        self.0.keys().copied().collect()
    }

    /// Adds one position and the anchor that owns it. Journal draining indexes
    /// newly written array and spill cells here; a stale extra entry only
    /// forces a conservative Full fallback, and the next full pass rebuilds the
    /// index exactly.
    pub(crate) fn insert(&mut self, cell: Position, anchor: Position) {
        self.0.insert(cell, anchor);
    }

    fn replace(&mut self, cells: HashMap<Position, Position>) {
        self.0 = cells;
    }
}

impl Shift for ArrayCells {
    fn shift(&mut self, displacement: Displacement) {
        self.0 = self
            .0
            .drain()
            .filter_map(|(cell, anchor)| {
                Some((displacement.position(cell)?, displacement.position(anchor)?))
            })
            .collect();
    }
}

/// The forward dependency graph and the pass state derived from it.
///
/// Edges are the reads observed while formulas evaluated, never a static
/// analysis of formula text, so the graph describes the pass that just ran.
/// Every index it holds is either rebuilt by a full pass or maintained across
/// an incremental one; anything it cannot represent is answered by forcing the
/// next pass full, which is what keeps incremental from diverging from full.
///
/// It stores no cell values. What it knows about stored state -- which cells
/// may not serve theirs -- arrives through `set_never_served` and
/// `set_blocked_array_readers`, rebuilt by `model::unstable_cells`.
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
    input_dependents: HashMap<Input, HashSet<Position>>,
    /// What each formula read last time it evaluated; the reverse of the edge
    /// maps, so a formula's edges can be dropped in O(degree) before re-record.
    precedents: HashMap<Position, ReadSet>,
    state: GraphState,
    /// The array/spill footprint index. Public to the crate because the
    /// scheduler tests membership directly to decide the arrays->Full fallback;
    /// `model::array_index` owns what goes into it.
    pub(crate) arrays: ArrayCells,
    /// Cells whose last result was not a genuine function value, because they
    /// sit on a dependency cycle, downstream of one, or reported `#CIRC!`. A
    /// cycle has no fixed point, so what they hold is an artifact of where the
    /// cycle was entered. Their stored value is never served: every pass seeds
    /// them dirty, so they and their readers recompute, exactly as a full pass
    /// re-derives them from scratch. Rebuilt after every pass by
    /// `Model::refresh_unstable_cells`.
    never_served: Positions,
    /// Readers of a blocked spill anchor: the other cells whose last result was
    /// not a function of the store. The anchor holds `#SPILL!` but hands a
    /// same-pass reader the live array's top-left value instead, so a reader
    /// recomputed against the stored error would not get what a full pass gets.
    /// Their stored value is served (it is what full computed, and it moves
    /// only when the anchor's cone moves), but recomputing one takes the full
    /// pass, which is the only pass that evaluates the anchor live. Rebuilt by
    /// `Model::refresh_blocked_array_readers` on every full pass; a blocked
    /// anchor can only appear or clear on one, because an evaluation write to
    /// an array footprint sends the pass to full.
    blocked_array_readers: Positions,
    /// Insert/delete can move data cells the dirty cone does not name. Cleared
    /// in [`Self::after_pass`] so it cannot leak across a Full fallback.
    structural_unknown: bool,
    /// The pass that just ran left values another full pass would still move: a
    /// spill or CSE footprint changed after something had already read it. Full
    /// mode heals that on its next unconditional pass, so incremental must run
    /// a full pass then too, or the two modes drift apart by exactly one
    /// evaluate. Survives [`Self::after_pass`]; the next pass consumes it
    /// through [`Self::take_convergence_debt`].
    convergence_debt: bool,
}

impl DependencyGraph {
    /// Drops all edges; a full pass rebuilds them. The never-served and
    /// blocked-reader sets are derived from those edges, so they go too.
    pub(crate) fn clear_edges(&mut self) {
        self.cell_dependents.clear();
        self.range_dependents.clear();
        self.input_dependents.clear();
        self.precedents.clear();
        self.never_served.replace(HashSet::new());
        self.blocked_array_readers.replace(HashSet::new());
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
    pub(crate) fn add_input_edge(&mut self, input: Input, dependent: Position) {
        self.input_dependents
            .entry(input)
            .or_default()
            .insert(dependent);
    }

    /// Formulas that read any input for which `pred` holds.
    pub(crate) fn dependents_of_inputs(&self, pred: impl Fn(&Input) -> bool) -> HashSet<Position> {
        self.input_dependents
            .iter()
            .filter(|(input, _)| pred(input))
            .flat_map(|(_, deps)| deps.iter().copied())
            .collect()
    }

    /// RAND/NOW/TODAY and CELL/INFO: re-roll every pass and always-report.
    pub(crate) fn always_dirty_cells(&self) -> HashSet<Position> {
        self.dependents_of_inputs(|i| {
            matches!(i, Input::Random | Input::Clock | Input::Environment)
        })
    }

    /// Replaces `dependent`'s outgoing edges with the reads just observed.
    pub(crate) fn replace_reads(&mut self, dependent: Position, reads: ReadSet) {
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

    /// Whether `cell`'s last evaluation recorded a non-cell input matching
    /// `pred`. Test-only: it reads an edge the graph otherwise only walks
    /// backwards.
    #[cfg(test)]
    pub(crate) fn cell_reads(&self, cell: Position, pred: impl Fn(&Input) -> bool) -> bool {
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

    /// Records that the pass that just ran is not a fixed point: another full
    /// pass over the same state would still move values. See
    /// [`Self::convergence_debt`].
    pub(crate) fn note_convergence_debt(&mut self) {
        self.convergence_debt = true;
    }

    /// Whether the previous pass left convergence debt, clearing the record.
    pub(crate) fn take_convergence_debt(&mut self) -> bool {
        std::mem::replace(&mut self.convergence_debt, false)
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

    /// Replaces the whole array index. Only a full pass may call this: it is
    /// the only pass whose walk sees every anchor, so it is the only one that
    /// can drop entries rather than just add them.
    pub(crate) fn replace_arrays(&mut self, cells: HashMap<Position, Position>) {
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

    /// The node set of the graph: every cell that has evaluated as a formula,
    /// plus every array footprint position. Only a formula reads anything, but
    /// a footprint cell relays its anchor's output to the formulas that read
    /// it, so a cycle can run through one.
    pub(crate) fn nodes(&self) -> HashSet<Position> {
        let mut nodes: HashSet<Position> = self.precedents.keys().copied().collect();
        nodes.extend(self.arrays.snapshot());
        nodes
    }

    /// Replaces the set of cells whose stored value may not be served. See
    /// [`Self::never_served`].
    pub(crate) fn set_never_served(&mut self, cells: HashSet<Position>) {
        self.never_served.replace(cells);
    }

    /// Cells whose last result was not a genuine function value. Seeded dirty
    /// on every incremental pass, and never compared against a re-evaluation
    /// that reads the store.
    pub(crate) fn never_served(&self) -> &HashSet<Position> {
        &self.never_served.0
    }

    /// Replaces the readers of blocked spill anchors. See
    /// [`Self::blocked_array_readers`].
    pub(crate) fn set_blocked_array_readers(&mut self, cells: HashSet<Position>) {
        self.blocked_array_readers.replace(cells);
    }

    /// Cells that read a blocked spill anchor: recomputing one takes a full
    /// pass, and no re-evaluation against the store reproduces it.
    pub(crate) fn blocked_array_readers(&self) -> &HashSet<Position> {
        &self.blocked_array_readers.0
    }

    /// Orders `affected` so each cell follows the affected cells it reads.
    /// Returns `Err` with the cells no order can place -- those on a dependency
    /// cycle plus everything downstream of one -- so the caller can fall back
    /// to the recursive recompute that reports `#CIRC!`, and so the cycle set
    /// can be rebuilt from the same walk.
    pub(crate) fn topo_order(
        &self,
        affected: &HashSet<Position>,
    ) -> Result<Vec<Position>, HashSet<Position>> {
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
        if order.len() == affected.len() {
            return Ok(order);
        }
        let ordered: HashSet<Position> = order.into_iter().collect();
        Err(affected.difference(&ordered).copied().collect())
    }

    /// The subset of `cells` lying on a dependency cycle or downstream of one:
    /// exactly what [`Self::topo_order`] cannot place. `cells` must be closed
    /// under dependents, so that every member of a cycle it touches is present
    /// and the answer is a whole cycle rather than a slice of one.
    pub(crate) fn cycle_cone(&self, cells: &HashSet<Position>) -> HashSet<Position> {
        self.topo_order(cells).err().unwrap_or_default()
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
        self.shift(Displacement {
            sheet,
            axis,
            boundary,
            delta,
        });
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
            Input::OwnCoord((s, ..)) | Input::FormulaText((s, ..)) => *s == sheet,
            Input::Name { .. } | Input::SheetStructure | Input::Computed => true,
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

    /// Rewrites every stored position for `displacement`. Edges and ranges
    /// landing in a deleted band are dropped; the next full pass rebuilds them.
    ///
    /// The destructuring is the mechanism, not a style choice: it names every
    /// field of [`DependencyGraph`], so a new index cannot be added without
    /// deciding here whether it shifts. Without it, an index that quietly keeps
    /// pre-edit coordinates is a silent wrong answer; with it, it is a compile
    /// error. Non-positional fields are bound to `_` with a reason.
    fn shift(&mut self, displacement: Displacement) {
        let Self {
            cell_dependents,
            range_dependents,
            input_dependents,
            precedents,
            state,
            arrays,
            never_served,
            blocked_array_readers,
            // Facts about the pass that just ran. They hold no coordinates.
            structural_unknown: _,
            convergence_debt: _,
        } = self;
        cell_dependents.shift(displacement);
        range_dependents.shift(displacement);
        input_dependents.shift(displacement);
        precedents.shift(displacement);
        state.shift(displacement);
        arrays.shift(displacement);
        never_served.shift(displacement);
        blocked_array_readers.shift(displacement);
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
