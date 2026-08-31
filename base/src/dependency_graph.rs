//! Dependency graph and recalculation mode for incremental evaluation.
//!
//! A forward dependency graph (precedent to dependents), rebuilt from the
//! reads observed while formulas evaluate, lets
//! [`Model::evaluate`](crate::Model::evaluate) recompute only the cells reachable
//! from those that changed. Anything the incremental path cannot model forces the
//! next pass to be full, so incremental never diverges from what full produces.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

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
/// A sheet id is allocated once at creation and never reassigned
/// (`Model::get_new_sheet_id`), so this sequence changes under exactly the edits
/// that renumber every stored `Position` and under no others: an insert or
/// delete resizes it, a move permutes it, and a rename — which changes formula
/// text, not numbering — leaves it alone. That is why the layout is *derived*
/// rather than counted. There is no generation to bump and so no bump to
/// forget: a new sheet-CRUD path is checked the moment it lands, without its
/// author knowing this type exists.
///
/// What the check is for is `base/src/recalc/README.md`, "Sheet numbering".
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

/// New coordinate after the line at `from` moves to `to`, the lines it passes
/// closing up behind it by one. Everything outside `[min(from, to),
/// max(from, to)]` stays where it is.
///
/// Total and injective, which is the whole reason a move can be modelled at
/// all: it neither creates nor destroys a line, so no stored entry is dropped
/// and no two entries collide. The band's own membership is unchanged — the
/// map permutes `[min, max]` onto itself — which is what lets a range holding
/// the whole band shift by identity.
fn move_coord(x: i32, from: i32, to: i32) -> i32 {
    if x == from {
        to
    } else if from < x && x <= to {
        x - 1
    } else if to <= x && x < from {
        x + 1
    } else {
        x
    }
}

/// How one structural edit rewrites the coordinates on its axis.
#[derive(Clone, Copy)]
enum Remap {
    /// An insert (`delta > 0`) or delete (`delta < 0`) of `|delta|` lines at
    /// `boundary`. Not injective: a delete maps a whole band to nothing.
    Band { boundary: i32, delta: i32 },
    /// One line moved from `from` to `to`. A permutation of the axis.
    ///
    /// Only ever one line, because that is the only move the model performs:
    /// `move_rows_action` decomposes a K-row move into K single-row moves and
    /// records a `DisplaceData::RowMove` for each, so the graph never sees a
    /// band move and does not have to model one.
    Move { from: i32, to: i32 },
}

/// One row/column structural edit on `sheet`, seen as a coordinate remapping.
///
/// Every coordinate the graph stores is rewritten through this one value, so
/// the remapping rule has a single definition. `None` from any of its methods
/// means the coordinate fell inside a deleted band: the entry holding it is
/// dropped, and the next full pass rebuilds it. A [`Remap::Move`] never
/// returns `None`.
#[derive(Clone, Copy)]
pub(crate) struct Displacement {
    sheet: u32,
    axis: Axis,
    remap: Remap,
}

impl Displacement {
    fn coord(self, x: i32) -> Option<i32> {
        match self.remap {
            Remap::Band { boundary, delta } => shift_coord(x, boundary, delta),
            Remap::Move { from, to } => Some(move_coord(x, from, to)),
        }
    }

    /// The new extent of the inclusive span `[a, b]` on this edit's axis, or
    /// `None` if it was deleted.
    ///
    /// A band edit maps the two ends and keeps them: the map is monotone, so
    /// the image of an interval is the interval between the images. A move's
    /// map is *not* monotone — the moved line jumps the band — so the image of
    /// an interval with one end inside the band and the other outside is an
    /// interval with a hole in it, which no single span can name. This returns
    /// the hull, deliberately: widening a stored range costs a redundant
    /// recompute, narrowing one drops a dependent and is a wrong answer. The
    /// move map is injective, so the hull can never come back narrower than
    /// the span it was given.
    fn span(self, a: i32, b: i32) -> Option<(i32, i32)> {
        match self.remap {
            Remap::Band { .. } => Some((self.coord(a)?, self.coord(b)?)),
            Remap::Move { from, to } => {
                // Away from `from` the map is `x + c` for a `c` that only ever
                // grows with `x`, so its extremes over `[a, b]` sit at the ends
                // of `[a, b]` or at one of the three lines around `from`, which
                // is the only place the map goes backwards. The two lines
                // around `to` are not candidates: the map is continuous in
                // *order* there — it steps from `to - 1` to `to + 1` — so an
                // interval spanning `to` still takes its extremes at its ends.
                let mut lo = i32::MAX;
                let mut hi = i32::MIN;
                for x in [a, b, from - 1, from, from + 1] {
                    if x < a || x > b {
                        continue;
                    }
                    let y = move_coord(x, from, to);
                    lo = lo.min(y);
                    hi = hi.max(y);
                }
                Some((lo, hi))
            }
        }
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
    /// [`DependencyGraph::structural_edit`] rebuilds instead. A move needs no
    /// such guard — see [`Self::span`].
    fn area(self, area: Area) -> Option<Area> {
        let (s, row1, column1, row2, column2) = area;
        if s != self.sheet {
            return Some(area);
        }
        match self.axis {
            Axis::Row => {
                let (row1, row2) = self.span(row1, row2)?;
                Some((s, row1, column1, row2, column2))
            }
            Axis::Column => {
                let (column1, column2) = self.span(column1, column2)?;
                Some((s, row1, column1, row2, column2))
            }
        }
    }

    /// The new form of a non-cell input. Only the position- and line-keyed
    /// variants move; the rest are keyed by nothing this edit touches.
    fn input(self, input: Input) -> Option<Input> {
        match input {
            Input::OwnCoord(p) => Some(Input::OwnCoord(self.position(p)?)),
            Input::FormulaText(a) => Some(Input::FormulaText(self.area(a)?)),
            Input::RowHidden(s, r1, r2) if s == self.sheet && matches!(self.axis, Axis::Row) => {
                let (r1, r2) = self.span(r1, r2)?;
                Some(Input::RowHidden(s, r1, r2))
            }
            Input::ColHidden(s, c1, c2) if s == self.sheet && matches!(self.axis, Axis::Column) => {
                let (c1, c2) = self.span(c1, c2)?;
                Some(Input::ColHidden(s, c1, c2))
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
        remap: Remap::Band { boundary, delta },
    }
    .position(pos)
}

/// The new location of `pos` after the line at `from` on `axis` moves to `to`
/// on `sheet`. Positions on other sheets are returned unchanged.
///
/// [`shift_position`]'s sibling for the move. The same rule the graph shifts
/// itself by, reached through the same [`Displacement`] so there is still one
/// definition of it; the `Option` is that shared signature and not a case a
/// move can produce, since a move deletes no line.
pub(crate) fn move_position(
    sheet: u32,
    axis: Axis,
    from: i32,
    to: i32,
    pos: Position,
) -> Option<Position> {
    Displacement {
        sheet,
        axis,
        remap: Remap::Move { from, to },
    }
    .position(pos)
}

/// A stored index whose coordinates move with a structural edit.
///
/// Every positional field of [`DependencyGraph`] implements this, and
/// [`DependencyGraph::shift`] applies it to each one by name. An index that
/// silently keeps pre-edit coordinates is a wrong answer with no failure to
/// notice it by; the destructuring in `shift` makes adding one without
/// deciding how it moves a compile error instead.
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

/// What one formula read when it last evaluated, and the whole-graph rebuild
/// that last confirmed it.
///
/// The stamp is what makes a full pass a *sweep* rather than a clear: see
/// [`DependencyGraph::begin_rebuild`].
#[derive(Clone, Debug, Default)]
struct Precedents {
    reads: ReadSet,
    /// The [`DependencyGraph::rebuild`] generation this entry was last written
    /// or re-confirmed in.
    seen: u64,
}

/// Knuth's multiplicative constant, 2^64 / phi rounded to an odd number.
const POSITION_HASH_MULTIPLIER: u64 = 0x9E37_79B9_7F4A_7C15;

/// The hash [`PrecedentStore`]'s index is built on: a multiply-xor mix of the
/// three words of a [`Position`].
///
/// The default `RandomState` is SipHash-1-3, whose reason to exist is
/// resistance to an adversary who chooses the keys. Nobody chooses these. They
/// are the coordinates of the cells a workbook holds, the map is private to
/// this module, and no key reaches it from outside the engine. What the default
/// costs instead is measurable: with the entry small enough to stay in cache,
/// hashing twelve bytes is about half of what the whole probe costs.
///
/// The accumulator is Fibonacci hashing --- multiplying by an odd constant is a
/// bijection on the low bits, so a column of consecutive rows lands spread
/// across the buckets rather than in a run. A multiply carries entropy upwards
/// only and `hashbrown` indexes its buckets from the *low* bits, so `finish`
/// folds the high half back down before handing the value over; without that
/// fold the top bits do all the work and the bucket index does none.
#[derive(Default)]
struct PositionHasher(u64);

impl Hasher for PositionHasher {
    fn finish(&self) -> u64 {
        let mut h = self.0;
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^ (h >> 29)
    }

    /// Every `write_*` below funnels into this one, so the mix is defined once.
    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0 ^ n).wrapping_mul(POSITION_HASH_MULTIPLIER);
    }

    fn write_u32(&mut self, n: u32) {
        self.write_u64(n as u64);
    }

    fn write_i32(&mut self, n: i32) {
        self.write_u64(n as u32 as u64);
    }

    /// The three calls `Hash for (u32, i32, i32)` makes are the two above; this
    /// is here because the trait requires it, and folds a byte at a time so
    /// that it is at least correct if some other key type ever arrives.
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(byte as u64);
        }
    }
}

/// What every formula that has evaluated read, keyed by the position that read
/// it.
///
/// Two structures rather than one map, and that split is the whole of the
/// design. [`DependencyGraph::replace_reads`] probes this once per formula per
/// pass --- on a full pass, once for every formula in the workbook, in the
/// order the pass walks them --- and that probe was the largest single term
/// left in a traced pass: 139 ns a formula on a 20,000-cell chain, against
/// 27 ns for the `ReadSet` comparison it exists to enable. A
/// `HashMap<Position, Precedents>` stores a 96-byte bucket, so that chain is a
/// 3 MB table, and hash order has nothing to do with walk order, so every probe
/// is a miss.
///
/// Splitting it puts the random access in a table small enough to stay in cache
/// --- 16 bytes a formula, half a megabyte at that size --- and the 80-byte
/// entries in a `Vec` addressed by a dense id. The ids are handed out in the
/// order positions first record, which on a full pass *is* the walk order, so
/// the second access is a sequential stride and not a second probe. Measured
/// against the single map and against boxing the read set, on the two shapes
/// that miss parity: 3.0x and 3.1x on the probe, where changing the hash alone
/// was 1.8x and 2.1x.
///
/// The lookup itself is not optional --- any scheme has to find a formula's
/// previous reads to know whether they moved --- so this makes it cheap rather
/// than avoiding it.
#[derive(Clone, Default)]
struct PrecedentStore {
    /// Position to the slot of `entries` holding its read set.
    slots: HashMap<Position, u32, BuildHasherDefault<PositionHasher>>,
    /// The read sets, by slot. A slot no position names holds
    /// `Precedents::default()` and is listed in `free`.
    entries: Vec<Precedents>,
    /// Slots to hand out before growing `entries`. Recycling is what keeps the
    /// ids dense over a model's life; without it a workbook that adds and
    /// removes formulas would grow `entries` forever.
    free: Vec<u32>,
}

impl PrecedentStore {
    /// Only the test-only readers of the graph want a whole entry; the pass
    /// itself takes [`Self::get_mut`], because it stamps what it finds.
    #[cfg(test)]
    fn get(&self, position: &Position) -> Option<&Precedents> {
        self.slots
            .get(position)
            .map(|&id| &self.entries[id as usize])
    }

    fn get_mut(&mut self, position: &Position) -> Option<&mut Precedents> {
        let &id = self.slots.get(position)?;
        Some(&mut self.entries[id as usize])
    }

    fn insert(&mut self, position: Position, precedents: Precedents) {
        if let Some(&id) = self.slots.get(&position) {
            self.entries[id as usize] = precedents;
            return;
        }
        let id = match self.free.pop() {
            Some(id) => {
                self.entries[id as usize] = precedents;
                id
            }
            None => {
                // One slot per formula ever live at once, and an entry is
                // eighty bytes, so reaching `u32::MAX` would take three hundred
                // gigabytes of read sets. Said out loud rather than left to be
                // a silent truncation if that ever stops being true.
                debug_assert!(self.entries.len() < u32::MAX as usize);
                self.entries.push(precedents);
                (self.entries.len() - 1) as u32
            }
        };
        self.slots.insert(position, id);
    }

    /// Drops `position`'s entry and returns it, freeing its slot. Clearing the
    /// entry is not tidiness: the slot is about to be handed to another
    /// position, and a `Vec` that kept the old read set would hand that
    /// position its predecessor's edges.
    fn remove(&mut self, position: &Position) -> Option<Precedents> {
        let id = self.slots.remove(position)?;
        self.free.push(id);
        Some(std::mem::take(&mut self.entries[id as usize]))
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.slots.len()
    }

    fn keys(&self) -> impl Iterator<Item = &Position> + '_ {
        self.slots.keys()
    }

    fn iter(&self) -> impl Iterator<Item = (&Position, &Precedents)> + '_ {
        self.slots
            .iter()
            .map(|(position, &id)| (position, &self.entries[id as usize]))
    }
}

impl Shift for PrecedentStore {
    fn shift(&mut self, displacement: Displacement) {
        let Self {
            slots,
            entries,
            free,
        } = self;
        let mut shifted = HashMap::with_capacity_and_hasher(slots.len(), Default::default());
        for (dependent, id) in std::mem::take(slots) {
            let Some(dependent) = displacement.position(dependent) else {
                // The position was deleted. Its slot goes back on the free
                // list, cleared, for the same reason `remove` clears one.
                entries[id as usize] = Precedents::default();
                free.push(id);
                continue;
            };
            // The entry keeps its slot and its rebuild stamp; only the
            // coordinates inside it move.
            let entry = &mut entries[id as usize];
            let reads = std::mem::take(&mut entry.reads);
            entry.reads = ReadSet {
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
            };
            shifted.insert(dependent, id);
        }
        *slots = shifted;
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
    precedents: PrecedentStore,
    /// Which whole-graph rebuild is running, or last ran. Bumped by
    /// [`Self::begin_rebuild`], stamped onto every entry [`Self::replace_reads`]
    /// writes or confirms, and read by [`Self::end_rebuild`] to sweep the rest.
    rebuild: u64,
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
    /// scratch. Derived from [`Self::cycle_cone`], over the whole graph after a
    /// full pass and over the cone after a selective one — and, because it is a
    /// function of the edges, only when they have moved. See
    /// [`Self::never_served_stale`].
    never_served: Positions,
    /// Whether [`Self::never_served`] may no longer be the whole-graph cycle
    /// cone of the edges now in hand.
    ///
    /// The cone is a function of the edges and of nothing else, so it only has
    /// to be re-derived when they move. Every mutator that moves one sets this;
    /// [`Self::refresh_never_served`] is the only thing that clears it, because
    /// it is the only thing that does the whole-graph derivation. A cone-shaped
    /// answer installed by a selective pass leaves it *set*, so the next full
    /// pass still does the whole-graph walk that "a cycle no cone would seed is
    /// still known" depends on.
    ///
    /// Sticky-true is the safe way for this to be wrong: an extra walk that
    /// finds what the last one found. Sticky-false would leave a cycle's
    /// members serving values that are artifacts of where the walk entered
    /// them, which is why it is set by the mutators rather than inferred at the
    /// point of use.
    never_served_stale: bool,
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
    /// How many times [`Self::cycle_cone`] has been asked to order a node set.
    /// Test-only, and for one thing: "a pass whose edges did not move did not
    /// order the graph again" is a statement about work *not done*, and no
    /// assertion over values or over [`Self::never_served`] can reach it — both
    /// are the same whether the walk ran or not.
    #[cfg(test)]
    cycle_scans: u64,
}

impl DependencyGraph {
    /// Opens a whole-graph rebuild: the full pass that follows re-records every
    /// live formula, and [`Self::end_rebuild`] drops whatever it did not.
    ///
    /// This is a mark and a sweep where the older shape was "clear every edge,
    /// then add them all back". The graph it leaves is the same one — what the
    /// pass recorded, entry for entry, because a full pass evaluates every cell
    /// in the workbook, so a position that is still a formula re-records and a
    /// position that is not cannot. What it no longer does is throw the read
    /// sets away first, and *that* is the point: clearing made every entry
    /// unrecognizable, so [`Self::replace_reads`] had to rebuild a whole
    /// workbook's edges from nothing on every full pass, even when nothing any
    /// formula reads had moved. With the sets still there, the formulas that
    /// read what they read last time — nearly all of them — cost a comparison
    /// instead.
    pub(crate) fn begin_rebuild(&mut self) {
        self.rebuild += 1;
        // `never_served` is *not* cleared here. It is derived from the edges,
        // and this pass mostly re-records the edges it already had; throwing it
        // away would mean deriving it again from an answer this graph already
        // holds. What stands in for the clear is `never_served_stale`, which
        // every mutator that actually moves an edge sets, so a pass that dies
        // mid-way leaves a set that is either still right or is marked as not.
        // That is strictly stronger than the clear was: an empty set over a
        // graph with a cycle in it is the dangerous direction, and the clear
        // produced exactly that until the tail ran.
        self.blocked_array_readers.replace(HashSet::new());
    }

    /// Closes the rebuild [`Self::begin_rebuild`] opened. An entry the pass
    /// neither rewrote nor confirmed belongs to a position that is no longer a
    /// formula, so its edges go with it.
    ///
    /// Must run before anything reads the edges — the cycle cone and the
    /// blocked-reader walk both do — because until it has, the graph is the
    /// union of this pass's edges and the leftovers of the last one.
    pub(crate) fn end_rebuild(&mut self) {
        let rebuild = self.rebuild;
        let stale: Vec<Position> = self
            .precedents
            .iter()
            .filter(|(_, precedents)| precedents.seen != rebuild)
            .map(|(&position, _)| position)
            .collect();
        for position in stale {
            self.remove_dependent(position);
        }
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
    ///
    /// A formula that read exactly what it read last time is the common case —
    /// on a full pass it is nearly every formula in the workbook — and for it
    /// the work below is a graph's worth of churn that ends where it began.
    /// [`Self::remove_dependent`] deletes every edge, dropping the dependents
    /// set of each precedent this was the last reader of, and the loops then
    /// allocate them all back. So compare first: one hash lookup and a scan
    /// linear in the degree, against a rebuild that is linear in the degree
    /// *and* allocates.
    ///
    /// Equivalence is the invariant, and it is exact rather than approximate.
    /// A remove followed by an identical add is the identity on all four maps:
    /// each edge is removed from the same set it is then inserted into, an
    /// entry pruned for becoming empty is recreated by the insert that follows,
    /// and `precedents` ends holding the value it started with. The one place
    /// the two loops differ — `remove_dependent` walks a self-read that
    /// `add_cell_edge` refuses to record — cancels too: nothing ever inserts
    /// `dependent` into its own dependents set, so removing it finds nothing
    /// and prunes nothing.
    ///
    /// The skip still stamps. A rebuild sweeps every entry it did not see, so
    /// an unchanged formula that returned here without saying so would have its
    /// edges collected as if the cell had stopped being a formula.
    pub(crate) fn replace_reads(&mut self, dependent: Position, reads: &ReadSet) {
        let rebuild = self.rebuild;
        if let Some(precedents) = self.precedents.get_mut(&dependent) {
            if precedents.reads == *reads {
                precedents.seen = rebuild;
                return;
            }
        }
        // Past here the edges move, and the cycle cone was derived from where
        // they were.
        self.never_served_stale = true;
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
        self.precedents.insert(
            dependent,
            Precedents {
                reads: reads.clone(),
                seen: rebuild,
            },
        );
    }

    /// Drops outgoing edges from a cell in O(degree) via the reverse index.
    pub(crate) fn remove_dependent(&mut self, dependent: Position) {
        let Some(Precedents { reads, .. }) = self.precedents.remove(&dependent) else {
            return;
        };
        self.never_served_stale = true;
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
            .is_some_and(|precedents| precedents.reads.inputs.iter().any(pred))
    }

    /// How many cell, rect and input edges `cell`'s last evaluation recorded.
    /// Test-only, and for one thing: a range walk must record a bounded number
    /// of edges whatever the height of the range it walked.
    #[cfg(test)]
    pub(crate) fn edge_counts(&self, cell: Position) -> (usize, usize, usize) {
        self.precedents
            .get(&cell)
            .map(|p| {
                (
                    p.reads.cells.len(),
                    p.reads.rects.len(),
                    p.reads.inputs.len(),
                )
            })
            .unwrap_or((0, 0, 0))
    }

    /// How many formulas the graph holds recorded reads for. Test-only, and for
    /// one thing: a full pass must leave an entry for the formulas it evaluated
    /// and for nothing else, which is a fact about the map's size that no value
    /// assertion can reach.
    #[cfg(test)]
    pub(crate) fn recorded_formula_count(&self) -> usize {
        self.precedents.len()
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

    /// Dirty cells and the cells reachable from them, or `None` once that set
    /// reaches `limit`.
    ///
    /// `limit` is the cone size at which the caller has *already* decided to
    /// run a full pass. A walk that reaches it therefore stops: what it would
    /// go on to build is a cone nobody reads, and on the shapes where every
    /// edit reaches every formula that unread cone is the largest single cost
    /// left in the pass.
    ///
    /// The dirty set is drained either way. The full pass that follows
    /// recomputes every cell those seeds could have reached and more, so
    /// dropping them is not losing them.
    pub(crate) fn take_seeds_and_affected(
        &mut self,
        limit: usize,
    ) -> Option<(Vec<Position>, HashSet<Position>)> {
        let GraphState::Ready { dirty } = &mut self.state else {
            return Some((Vec::new(), HashSet::new()));
        };
        let seeds: Vec<Position> = std::mem::take(dirty).into_iter().collect();
        let affected = self.reachable_within(seeds.clone(), limit)?;
        Some((seeds, affected))
    }

    /// Replaces the whole array index. Only a full pass may call this: it is
    /// the only pass whose walk sees every anchor, so it is the only one that
    /// can drop entries rather than just add them.
    pub(crate) fn replace_arrays(&mut self, cells: HashMap<Position, Position>) {
        // Footprint positions are nodes of the cycle graph -- a member relays
        // its anchor's output -- so a different index is a different node set.
        // Compared rather than assumed changed: a workbook with arrays in it
        // rebuilds this index on every full pass and almost always rebuilds it
        // to what it already was, and paying a whole-graph walk for that would
        // be exactly the cost this fact exists to avoid.
        if self.arrays.0 != cells {
            self.never_served_stale = true;
        }
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

    /// Installs a *cone-shaped* answer: what a selective pass found by ordering
    /// its own cone. See [`Self::never_served`].
    ///
    /// Leaves [`Self::never_served_stale`] set, deliberately. A cone is not the
    /// whole graph, and the reason the whole-graph walk exists is that a cycle
    /// no cone would seed still has to be known; so this answer stands until
    /// the next full pass, and does not excuse that pass from its walk.
    pub(crate) fn set_never_served(&mut self, cells: HashSet<Position>) {
        self.never_served.replace(cells);
        self.never_served_stale = true;
    }

    /// Re-derives [`Self::never_served`] over the whole graph -- unless the
    /// edges have not moved since the last time it was derived that way, in
    /// which case the set already in hand *is* that answer.
    ///
    /// This is the one place the whole-graph walk happens, so it is the one
    /// place that may clear the staleness fact. Everything else that touches
    /// edges, footprints or the set itself only ever sets it.
    pub(crate) fn refresh_never_served(&mut self) {
        if !self.never_served_stale {
            return;
        }
        let nodes = self.nodes();
        let cone = self.cycle_cone(&nodes);
        self.never_served.replace(cone);
        self.never_served_stale = false;
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
        // produces is the one a sorted walk over `Position` keys produces,
        // without hashing a 12-byte tuple per edge, twice.
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
    ///
    /// Takes `&mut self` for [`Self::cycle_scans`] alone — it reads the graph
    /// and changes nothing in it. Both callers hold `&mut` already, and a `Cell`
    /// here would buy back a `&self` nobody needs at the price of making the
    /// graph's interior mutable for a tally.
    pub(crate) fn cycle_cone(&mut self, cells: &HashSet<Position>) -> HashSet<Position> {
        #[cfg(test)]
        {
            self.cycle_scans += 1;
        }
        self.topo_order(cells).err().unwrap_or_default()
    }

    /// How many node sets [`Self::cycle_cone`] has been asked to order. See
    /// [`Self::cycle_scans`].
    #[cfg(test)]
    pub(crate) fn cycle_scans(&self) -> u64 {
        self.cycle_scans
    }

    /// Every cell transitively reachable from `seeds`, including the seeds, with
    /// no limit on how many that is. Does not touch the dirty set.
    ///
    /// The scheduler does not want this — it always has a size past which it
    /// stops caring, and takes [`Self::take_seeds_and_affected`]. `Verify` is
    /// the one caller that wants the whole cone: it passes the RAND/NOW/TODAY
    /// seeds and strips what they reach from its value comparison, and a cone
    /// cut short there would be a comparison made against cells it should have
    /// skipped.
    ///
    /// The walk stops early only at `limit`, and a set of `usize::MAX`
    /// positions does not fit in a machine, so the `None` this defaults away is
    /// unreachable. Defaulting rather than unwrapping is still the right way to
    /// be wrong here: `Verify` subtracts this set from what it compares, so an
    /// empty one makes it compare cells it meant to skip and fail loudly, where
    /// a wrong non-empty one would make it skip cells it meant to compare and
    /// say nothing.
    #[cfg(feature = "recalc_verify")]
    pub(crate) fn reachable(&self, seeds: Vec<Position>) -> HashSet<Position> {
        self.reachable_within(seeds, usize::MAX).unwrap_or_default()
    }

    /// [`Self::reachable`], abandoned the moment the set reaches `limit`.
    ///
    /// The check is on the set rather than on the stack, so it counts cells and
    /// not the edges that led to them: a walk over a wide fanout pushes each
    /// cell once per precedent, and stopping on that count would stop at a
    /// number that depends on the shape of the graph rather than on the size of
    /// the cone the caller asked about.
    fn reachable_within(&self, seeds: Vec<Position>, limit: usize) -> Option<HashSet<Position>> {
        let mut affected = HashSet::new();
        let mut stack = seeds;
        // A range's dependents all become affected the moment any one of its
        // cells does, so fire each range at most once and drop it from the scan.
        let mut fired: HashSet<Area> = HashSet::new();
        while let Some(cell) = stack.pop() {
            if !affected.insert(cell) {
                continue;
            }
            if affected.len() >= limit {
                return None;
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
        Some(affected)
    }

    /// Applies a row/column insert (`delta > 0`) or delete (`delta < 0`): marks
    /// the dependents the edit can change, then shifts every stored position.
    pub(crate) fn structural_edit(&mut self, sheet: u32, axis: Axis, boundary: i32, delta: i32) {
        // A delete that shrinks a tracked range would need the range clamped.
        if delta < 0 && self.range_overlaps_band(sheet, axis, boundary, delta) {
            self.state = GraphState::MustRebuild;
            return;
        }
        // Everything at or after the boundary moves, so the band is open-ended.
        self.apply(
            sheet,
            axis,
            Remap::Band { boundary, delta },
            (boundary, i32::MAX),
        );
    }

    /// Applies a row/column move of the line at `from` to `to`.
    ///
    /// The move needs no counterpart to `structural_edit`'s shrink guard. That
    /// guard exists because a delete can take rows out of the middle of a
    /// tracked range, which two shifted corners cannot express; the move map is
    /// a permutation, so [`Displacement::span`] always has a hull to return and
    /// it is never narrower than what it was given.
    pub(crate) fn structural_move(&mut self, sheet: u32, axis: Axis, from: i32, to: i32) {
        // Only the lines between the two ends move; the rest is identity.
        let band = (from.min(to), from.max(to));
        self.apply(sheet, axis, Remap::Move { from, to }, band);
    }

    /// The body both structural edits share: mark what the edit can change,
    /// rewrite every stored coordinate, and record that the delta cannot name
    /// the data cells that moved.
    fn apply(&mut self, sheet: u32, axis: Axis, remap: Remap, band: (i32, i32)) {
        if !matches!(self.state, GraphState::Ready { .. }) {
            self.state = GraphState::MustRebuild;
            return;
        }
        self.mark_structural_dependents(sheet, axis, band);
        self.shift(Displacement { sheet, axis, remap });
        // Data cells in the shift band are not dirty. The cell-list delta cannot
        // name them, so `take_changed_cells` reports Everything after this pass.
        self.structural_unknown = true;
    }

    /// Marks the dependents a structural edit can change: those reading a
    /// precedent inside `band` — the inclusive span of coordinates the edit
    /// moves — or a range reaching into it. Uses pre-shift coordinates;
    /// [`shift`](Self::shift) then moves the dirty set with the rest.
    fn mark_structural_dependents(&mut self, sheet: u32, axis: Axis, (lo, hi): (i32, i32)) {
        if !matches!(self.state, GraphState::Ready { .. }) {
            return;
        }
        let mut extra: HashSet<Position> = HashSet::new();
        for (precedent, dependents) in &self.cell_dependents {
            if precedent.0 == sheet && (lo..=hi).contains(&axis.coord(*precedent)) {
                extra.extend(dependents.iter().copied());
            }
        }
        if let Some(sheet_ranges) = self.range_dependents.get(&sheet) {
            for (area, dependents) in sheet_ranges.iter() {
                if axis.area_max(*area) >= lo && axis.area_min(*area) <= hi {
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
            // A fact about whether the cycle cone still describes these edges.
            // It holds no coordinates, and it is set unconditionally at the end
            // of this function — see the note there.
            never_served_stale: _,
            // A counter of rebuilds. `precedents` carries the stamp with the
            // entry it belongs to, so the entries a shift moves keep the one
            // they had and the entries it drops take theirs with them.
            rebuild: _,
            // Sheet ids, not coordinates. A row/column edit happens *within* one
            // sheet and cannot add, remove or reorder sheets, so the numbering
            // this names is exactly the one it named before.
            sheet_layout: _,
            // A tally of scans, in test builds only. It holds no coordinates.
            #[cfg(test)]
                cycle_scans: _,
        } = self;
        cell_dependents.shift(displacement);
        range_dependents.shift(displacement);
        input_dependents.shift(displacement);
        precedents.shift(displacement);
        state.shift(displacement);
        arrays.shift(displacement);
        never_served.shift(displacement);
        blocked_array_readers.shift(displacement);
        // The cone is derived again after this rather than shifted into place.
        //
        // Every other setter of this fact is reached by a mutator; this one is
        // not, because a displacement drops the entries whose positions fell in
        // a deleted band by rewriting the map, never through
        // `remove_dependent`. There is an argument that it is redundant anyway
        // -- removing a cell that a cycle ran through breaks a reference some
        // survivor held, and that survivor re-records different reads on the
        // next pass, which sets the fact through `replace_reads`. No test kills
        // this line, and that argument is why.
        //
        // It stays because the argument leans on the shift model being exact,
        // and this fact is what stands between a wrong shift and a cycle whose
        // members quietly stop being seeded. One bool on the rarest path is a
        // cheaper thing to keep than that argument is to rely on.
        self.never_served_stale = true;
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
        let Some((_seeds, affected)) = graph.take_seeds_and_affected(usize::MAX) else {
            panic!("an unlimited walk cannot abandon the cone");
        };
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
                remap: Remap::Band { boundary, delta },
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
                remap: Remap::Band {
                    boundary: 5,
                    delta: 2
                },
            }
            .position((0, 5, 1)),
            Some((0, 5, 1))
        );

        // An area is dropped when either edge was deleted, and only then.
        let area = |boundary, delta, a| {
            Displacement {
                sheet: 0,
                axis: Axis::Row,
                remap: Remap::Band { boundary, delta },
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

    /// I5.6 — a move remaps a cell by permutation, and widens a range it
    /// cannot name rather than narrowing it.
    ///
    /// Two halves, because the map and the hull fail differently. The map is
    /// pinned by value: it is the same rule `to_string_displaced` rewrites
    /// formula text by, and a graph disagreeing with the text would hold
    /// coordinates the next evaluation never reads. Kills the map collapsing to
    /// the identity, and kills widening or narrowing any of its comparisons —
    /// each of the four band edges is checked from both sides.
    ///
    /// The hull is the range decision, brute-forced against the actual image
    /// because "covers" cannot be stated any other way without restating the
    /// arithmetic. A range with one end inside the moved band and the other
    /// outside has an image with a hole in it, which no single span names; the
    /// two failure modes are not symmetric, so the answer widens. Kills
    /// remapping the two corners the way the band arm does (`(coord(a),
    /// coord(b))` shrinks `A1:A5` to `A1:A4` under a 3 -> 7 move, and inverts
    /// it under 3 -> 4), and kills dropping any of the three lines around
    /// `from` from the candidate list — each is the sole extreme for some
    /// range, which is also why the three lines around `to` are not there.
    #[test]
    fn move_remaps_by_permutation_and_widens_what_it_cannot_name() {
        // Row 3 moves down to row 7: 3 lands on 7, rows 4..7 close up by one,
        // and the lines outside the band do not move.
        assert_eq!(
            (2..=8).map(|x| move_coord(x, 3, 7)).collect::<Vec<_>>(),
            vec![2, 7, 3, 4, 5, 6, 8]
        );
        // Row 7 moves up to row 3: the exact inverse, which is what makes the
        // map a permutation rather than a shift.
        assert_eq!(
            (2..=8).map(|x| move_coord(x, 7, 3)).collect::<Vec<_>>(),
            vec![2, 4, 5, 6, 7, 3, 8]
        );
        // A line moved onto itself moves nothing.
        assert_eq!(
            (1..=5).map(|x| move_coord(x, 3, 3)).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );

        for from in 1..=9 {
            for to in 1..=9 {
                let displacement = Displacement {
                    sheet: 0,
                    axis: Axis::Row,
                    remap: Remap::Move { from, to },
                };
                for a in 1..=9 {
                    for b in a..=9 {
                        let (lo, hi) = displacement.span(a, b).unwrap();
                        let image: Vec<i32> = (a..=b).map(|x| move_coord(x, from, to)).collect();
                        let (want_lo, want_hi) =
                            (*image.iter().min().unwrap(), *image.iter().max().unwrap());
                        assert_eq!(
                            (lo, hi),
                            (want_lo, want_hi),
                            "[{a},{b}] under {from}->{to}: image {image:?}"
                        );
                        // The move destroys no line, so the hull of the image of
                        // n lines holds at least n lines. Widening is the only
                        // direction this can be wrong in.
                        assert!(hi - lo >= b - a, "[{a},{b}] shrank to [{lo},{hi}]");
                    }
                }
            }
        }
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

    /// I5.7 — a slot the store hands out again carries none of its last
    /// occupant's reads.
    ///
    /// [`PrecedentStore`] addresses read sets by a dense id and recycles the
    /// ids, which is a way of being wrong the single map it replaced did not
    /// have: a formula landing on a stale slot would answer with the previous
    /// occupant's precedents, and `remove_dependent` would then delete edges
    /// belonging to a cell that no longer exists. Kills dropping the clear in
    /// [`PrecedentStore::remove`], and kills dropping the one on the shift
    /// path, which frees its slots separately and so has its own clear.
    #[test]
    fn a_recycled_slot_carries_no_reads_from_its_last_occupant() {
        let reads = |cell: Position| ReadSet {
            cells: vec![cell],
            rects: Vec::new(),
            inputs: Vec::new(),
        };
        let mut graph = DependencyGraph::default();
        graph.replace_reads((0, 1, 2), &reads((0, 1, 1)));
        graph.replace_reads((0, 2, 2), &reads((0, 2, 1)));

        // The first cell stops being a formula, so its slot is handed back.
        graph.remove_dependent((0, 1, 2));
        assert!(graph.precedents.get(&(0, 1, 2)).is_none());
        assert_eq!(graph.precedents.free.len(), 1);

        // A new formula takes that slot and must not inherit what was in it.
        graph.replace_reads((0, 3, 2), &reads((0, 3, 1)));
        assert!(graph.precedents.free.is_empty());
        assert_eq!(
            graph.precedents.get(&(0, 3, 2)).unwrap().reads,
            reads((0, 3, 1))
        );
        assert_eq!(
            graph.precedents.get(&(0, 2, 2)).unwrap().reads,
            reads((0, 2, 1))
        );
        assert_eq!(graph.dependents_of((0, 3, 1)), vec![(0, 3, 2)]);
        assert!(graph.dependents_of((0, 1, 1)).is_empty());

        // The other way a slot comes free: a structural edit deletes the
        // position that held it, by rewriting the map rather than by calling
        // `remove`.
        graph.after_pass();
        graph.structural_edit(0, Axis::Row, 3, -1);
        assert!(graph.precedents.get(&(0, 3, 2)).is_none());
        assert_eq!(graph.precedents.free.len(), 1);
        graph.replace_reads((0, 9, 2), &reads((0, 9, 1)));
        assert_eq!(
            graph.precedents.get(&(0, 9, 2)).unwrap().reads,
            reads((0, 9, 1))
        );
        assert_eq!(graph.dependents_of((0, 9, 1)), vec![(0, 9, 2)]);
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
