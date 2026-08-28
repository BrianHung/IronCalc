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

/// The sheet numbering every stored [`Position`] is expressed in: the workbook's
/// sheet ids, in workbook order.
///
/// A `Position`'s sheet component is an *index* into `workbook.worksheets`, so
/// adding, deleting, duplicating or moving a sheet renumbers every position the
/// graph holds — silently, because the old index still names a live sheet. The
/// convention that stops that is `reset_parsed_structures` calling
/// `invalidate_graph`, and until this type existed nothing checked it.
///
/// A sheet id is allocated once at creation and never reassigned
/// (`Model::get_new_sheet_id`), so this sequence changes under exactly the edits
/// that renumber and under no others: an insert or delete resizes it, a move
/// permutes it, and a rename — which changes formula text, not numbering —
/// leaves it alone. That is why the layout is *derived* rather than counted.
/// There is no generation to bump and so no bump to forget: a new sheet-CRUD
/// path is checked the moment it lands, without knowing this type exists.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub(crate) struct SheetLayout(Vec<u32>);

impl SheetLayout {
    /// The layout of a workbook whose sheets have these ids, in workbook order.
    pub(crate) fn from_sheet_ids(ids: impl IntoIterator<Item = u32>) -> Self {
        SheetLayout(ids.into_iter().collect())
    }
}

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

    /// How many distinct ranges are read on this sheet.
    fn len(&self) -> usize {
        self.dependents.len()
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
/// may not serve theirs -- arrives through two setters that differ in where
/// the answer comes from. `set_never_served` takes a [`Self::cycle_cone`] the
/// graph computed from its own edges; each scheduler installs it after its
/// pass, because only the scheduler knows which cells that pass covered.
/// `set_blocked_array_readers` takes a set only `model::unstable_cells` can
/// build, because deciding an anchor is blocked means reading what it stores.
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
    /// sit on a dependency cycle or downstream of one. A cycle has no fixed
    /// point, so what they hold is an artifact of where the cycle was entered.
    /// Their stored value is never served: every pass seeds them dirty, so they
    /// and their readers recompute, exactly as a full pass re-derives them from
    /// scratch. Rebuilt after every pass from [`Self::cycle_cone`], over the
    /// whole graph after a full pass and over the cone after a selective one.
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
    /// The sheet numbering the stored positions are expressed in, as of the last
    /// pass. Compared against the workbook's current layout at every pass entry
    /// by [`Self::sync_sheet_layout`]; a disagreement means sheet CRUD moved the
    /// coordinates out from under the edges.
    sheet_layout: SheetLayout,
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

    /// The successors of every node, as dense ids into `nodes`: exactly what
    /// [`Self::dependents_of`] returns for each of them, restricted to `nodes`,
    /// sorted, duplicates kept (a dependent that reads a cell *and* a range over
    /// it is two edges, and the walk counts it twice either way).
    ///
    /// Two ways to get there, and the point of this method is that they cost
    /// differently. `dependents_of` is a point query: it scans the row band of
    /// the range index that the cell falls in, which is right for a cone of a
    /// few cells and ruinous for a whole-workbook set -- a band holds every
    /// range overlapping 256 rows, so a workbook of overlapping windowed `SUM`s
    /// pays hundreds of rejected containment tests per cell, once per cell in
    /// the workbook. Walking the ranges instead visits each one once and hands
    /// it to the cells it covers, which `nodes` being sorted makes a slice.
    ///
    /// The strategy is chosen by which side is bigger. Both produce the same
    /// lists; this decides nothing but the cost.
    fn successors_within(&self, nodes: &[Position], ids: &HashMap<Position, u32>) -> Vec<Vec<u32>> {
        let mut successors: Vec<Vec<u32>> = vec![Vec::new(); nodes.len()];
        let range_count: usize = self.range_dependents.values().map(SheetRanges::len).sum();
        if nodes.len() < range_count {
            for (id, &cell) in nodes.iter().enumerate() {
                successors[id].extend(
                    self.dependents_of(cell)
                        .into_iter()
                        .filter_map(|dependent| ids.get(&dependent).copied()),
                );
            }
        } else {
            for (precedent, dependents) in &self.cell_dependents {
                let Some(&id) = ids.get(precedent) else {
                    continue;
                };
                successors[id as usize].extend(
                    dependents
                        .iter()
                        .filter_map(|dependent| ids.get(dependent).copied()),
                );
            }
            for (&sheet, sheet_ranges) in &self.range_dependents {
                for (&(_, row1, column1, row2, column2), dependents) in sheet_ranges.iter() {
                    let inside: Vec<u32> = dependents
                        .iter()
                        .filter_map(|dependent| ids.get(dependent).copied())
                        .collect();
                    if inside.is_empty() {
                        continue;
                    }
                    // `nodes` is sorted, so this sheet's rows within the range
                    // are one contiguous slice and only the column is tested per
                    // cell. A whole-column reference costs the cells it covers,
                    // not the million rows it names.
                    let first = nodes.partition_point(|&(s, row, _)| (s, row) < (sheet, row1));
                    for (offset, &(s, row, column)) in nodes[first..].iter().enumerate() {
                        if s != sheet || row > row2 {
                            break;
                        }
                        if column >= column1 && column <= column2 {
                            successors[first + offset].extend(inside.iter().copied());
                        }
                    }
                }
            }
        }
        for dependents in &mut successors {
            dependents.sort_unstable();
        }
        successors
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
        // Dense ids assigned in `Position` order, so the walk is index
        // arithmetic and ascending id *is* ascending position: the order this
        // produces is the one a sorted walk over `Position` keys produces, and
        // a full pass no longer hashes a 12-byte tuple per edge, twice.
        let mut nodes: Vec<Position> = affected.iter().copied().collect();
        nodes.sort_unstable();
        let ids: HashMap<Position, u32> = nodes
            .iter()
            .enumerate()
            .map(|(id, &position)| (position, id as u32))
            .collect();
        let successors = self.successors_within(&nodes, &ids);

        let mut indegree: Vec<u32> = vec![0; nodes.len()];
        for dependents in &successors {
            for &dependent in dependents {
                indegree[dependent as usize] += 1;
            }
        }
        let mut queue: Vec<u32> = (0..nodes.len() as u32)
            .filter(|&id| indegree[id as usize] == 0)
            .collect();
        let mut order = Vec::with_capacity(nodes.len());
        let mut head = 0;
        while head < queue.len() {
            let id = queue[head];
            head += 1;
            order.push(nodes[id as usize]);
            for &dependent in &successors[id as usize] {
                let remaining = &mut indegree[dependent as usize];
                *remaining -= 1;
                if *remaining == 0 {
                    queue.push(dependent);
                }
            }
        }
        if order.len() == nodes.len() {
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
            // A fact about the pass that just ran. It holds no coordinates.
            structural_unknown: _,
            // Sheet ids, not coordinates. A row/column edit happens *within* one
            // sheet and cannot add, remove or reorder sheets, so the numbering
            // this names is exactly the one it named before.
            sheet_layout: _,
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

    /// Adopts `layout` as the numbering the stored positions are in, reporting
    /// whether that numbering had changed underneath a graph still holding
    /// edges — a sheet added, deleted, duplicated or moved without the
    /// `invalidate_graph` that every such edit is supposed to make.
    ///
    /// Detection, not repair, is the point: a `Ready` graph whose layout moved
    /// holds positions whose sheet component now names a *different live sheet*,
    /// so every edge, precedent, array entry and never-served cell in it is a
    /// silently wrong answer waiting to be read. The graph is downgraded to
    /// `MustRebuild` here so release builds degrade to a correct full pass
    /// instead of serving that corruption, and the caller raises a
    /// `debug_assert` so a debug or test build fails loudly at the edit that
    /// skipped the invalidation rather than at whatever later reads it.
    ///
    /// A `MustRebuild` graph holds no positions, so a layout change against one
    /// is not staleness — it is the ordinary first pass, or the pass after any
    /// correctly-invalidated sheet edit — and reports `false`.
    pub(crate) fn sync_sheet_layout(&mut self, layout: SheetLayout) -> bool {
        let stale = matches!(self.state, GraphState::Ready { .. }) && self.sheet_layout != layout;
        self.sheet_layout = layout;
        if stale {
            self.state = GraphState::MustRebuild;
        }
        stale
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

    /// I8.10 — a sheet renumbering under a graph that still holds edges is
    /// detected, and only that.
    ///
    /// The check is two one-sided conditions and this pins both. Kills
    /// `sync_sheet_layout` always answering `false` (the mechanism dead, and
    /// sheet CRUD back to silently pointing every stored coordinate at the
    /// wrong sheet); kills dropping the `Ready` guard (every first pass would
    /// then report staleness, so the panic would fire on correct programs);
    /// kills comparing the layouts with `==` instead of `!=`; and kills
    /// detecting without downgrading, which is what makes release builds fall
    /// back to a correct full pass instead of walking the stale edges.
    #[test]
    fn sheet_renumbering_under_a_ready_graph_is_detected() {
        let layout = |ids: &[u32]| SheetLayout::from_sheet_ids(ids.to_vec());
        let mut graph = DependencyGraph::default();

        // MustRebuild holds no positions, so adopting any layout is not
        // staleness — this is the ordinary first pass.
        assert!(!graph.sync_sheet_layout(layout(&[1, 2])));
        graph.after_pass();
        graph.mark_dirty((0, 1, 1));

        // Same layout, same numbering: the graph stays walkable.
        assert!(!graph.sync_sheet_layout(layout(&[1, 2])));
        assert!(!graph.should_recompute_full());

        // A sheet deleted. The stored `(0, ..)` and `(1, ..)` now mean
        // different sheets than they did, so the edges cannot be walked.
        assert!(graph.sync_sheet_layout(layout(&[2])));
        assert!(graph.should_recompute_full());
        assert!(graph.full_reflects_change());

        // A *move* renumbers without changing the sheet count, which is why the
        // layout is the id sequence and not its length.
        let mut graph = DependencyGraph::default();
        graph.sync_sheet_layout(layout(&[1, 2]));
        graph.after_pass();
        assert!(graph.sync_sheet_layout(layout(&[2, 1])));

        // A rename changes formula text, not numbering: the ids are untouched,
        // so this mechanism stays silent and the reparse's own invalidation is
        // what covers it.
        let mut graph = DependencyGraph::default();
        graph.sync_sheet_layout(layout(&[1, 2]));
        graph.after_pass();
        assert!(!graph.sync_sheet_layout(layout(&[1, 2])));
    }

    /// I5.5 — the remapping rule is exact at the edit boundary.
    ///
    /// `shift_coord` is the single definition every stored coordinate is
    /// rewritten through, so its two comparisons are the whole rule and both
    /// are one-sided. Kills widening either of them: `x < boundary` to `<=`
    /// (the line *at* an insert boundary must move, or an index keeps a
    /// pre-edit coordinate the next full pass is not there to repair), and
    /// `x < boundary - delta` to `<=` (the first line *below* a deleted band
    /// survives; dropping it silently deletes a live edge).
    #[test]
    fn displacement_remaps_at_the_edit_boundary() {
        let at = |axis, boundary, delta, pos| {
            Displacement {
                sheet: 0,
                axis,
                boundary,
                delta,
            }
            .position(pos)
        };

        // Insert of two rows at row 5. Row 5 itself is inside the shift.
        assert_eq!(at(Axis::Row, 5, 2, (0, 4, 1)), Some((0, 4, 1)));
        assert_eq!(at(Axis::Row, 5, 2, (0, 5, 1)), Some((0, 7, 1)));
        assert_eq!(at(Axis::Row, 5, 2, (0, 6, 1)), Some((0, 8, 1)));

        // Delete of two rows at row 5: rows 5 and 6 go, row 7 becomes row 5.
        assert_eq!(at(Axis::Row, 5, -2, (0, 4, 1)), Some((0, 4, 1)));
        assert_eq!(at(Axis::Row, 5, -2, (0, 5, 1)), None);
        assert_eq!(at(Axis::Row, 5, -2, (0, 6, 1)), None);
        assert_eq!(at(Axis::Row, 5, -2, (0, 7, 1)), Some((0, 5, 1)));

        // The same rule on the other axis, and never on another sheet.
        assert_eq!(at(Axis::Column, 3, 1, (0, 9, 3)), Some((0, 9, 4)));
        assert_eq!(at(Axis::Column, 3, -1, (0, 9, 3)), None);
        assert_eq!(
            Displacement {
                sheet: 1,
                axis: Axis::Row,
                boundary: 5,
                delta: 2,
            }
            .position((0, 5, 1)),
            Some((0, 5, 1))
        );

        // An area is dropped when either edge was deleted, and only then.
        let area = |boundary, delta, a| {
            Displacement {
                sheet: 0,
                axis: Axis::Row,
                boundary,
                delta,
            }
            .area(a)
        };
        assert_eq!(area(5, 2, (0, 5, 1, 6, 1)), Some((0, 7, 1, 8, 1)));
        assert_eq!(area(5, -2, (0, 7, 1, 9, 1)), Some((0, 5, 1, 7, 1)));
        // A corner inside the deleted band drops the entry.
        assert_eq!(area(5, -2, (0, 6, 1, 9, 1)), None);
        // A range that merely *straddles* the band keeps both corners and so
        // silently shrinks. That is why `range_overlaps_band` has to reject the
        // edit before `shift` ever runs: this arithmetic cannot represent it.
        assert_eq!(area(5, -2, (0, 4, 1, 7, 1)), Some((0, 4, 1, 5, 1)));
    }

    /// I5.3 — shrink detection is exact at both edges of the deleted band.
    ///
    /// A delete that only *shrinks* a tracked range cannot be modeled by
    /// shifting its two corners, so the graph rebuilds instead. The test is an
    /// interval overlap and both of its ends are one-sided. Kills widening
    /// `area_min <= band_end` (which would rebuild for a range starting just
    /// below the band — a pass wasted on every delete above a range) via
    /// `band_end = boundary - delta - 1` losing its `- 1`, and kills narrowing
    /// `area_max >= boundary` to `>` or `area_min <= band_end` to `<`, either
    /// of which shifts a range whose own edge was deleted: `Displacement::area`
    /// then returns `None` and the range edge disappears without a rebuild.
    #[test]
    fn shrink_detection_is_exact_at_the_band_edges() {
        let mut graph = DependencyGraph::default();
        let dependent: Position = (0, 100, 100);
        graph.add_range_edge((0, 2, 1, 4, 1), dependent); // A2:A4

        // Deleting the range's last row shrinks it: rebuild.
        assert!(graph.range_overlaps_band(0, Axis::Row, 4, -1));
        // Deleting the range's first row shrinks it too.
        assert!(graph.range_overlaps_band(0, Axis::Row, 2, -1));
        // Wholly below the range: the range does not move at all.
        assert!(!graph.range_overlaps_band(0, Axis::Row, 5, -2));
        // Wholly above it: the range shifts up intact, corners and all.
        assert!(!graph.range_overlaps_band(0, Axis::Row, 1, -1));
        // Rows 1 and 2 with the range at A3:A5: the band stops one row short
        // of the range, so this is still a whole-range shift.
        let mut graph = DependencyGraph::default();
        graph.add_range_edge((0, 3, 1, 5, 1), dependent);
        assert!(!graph.range_overlaps_band(0, Axis::Row, 1, -2));
        // One more deleted row and it does reach the range.
        assert!(graph.range_overlaps_band(0, Axis::Row, 1, -3));
        // The axis and the sheet both have to match.
        assert!(!graph.range_overlaps_band(0, Axis::Column, 3, -1));
        assert!(!graph.range_overlaps_band(1, Axis::Row, 3, -1));
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
