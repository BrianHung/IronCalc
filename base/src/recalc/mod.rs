//! Incremental recalculation: evaluating only the formulas an edit can change.
//!
//! There is one evaluation, the one in `evaluation.rs`, and it runs the
//! formulas it is given. What it is given is decided here, in `plan`: every
//! formula, or a *scope*, the formulas an edit can have changed. Any other
//! formula holds what the last evaluation stored, and a formula reading it
//! gets that. In short:
//!
//! * While a formula runs, the cells and the rectangles it reads are put on
//!   record. When it stores its result, they replace what it read the last
//!   time in the dependency graph (`graph.rs`).
//! * Every sheet logs what is written to it (`sheet_data::Writes`): a cell, a
//!   whole row, rows or columns moving. `plan` takes the logs in order. A
//!   written position is a seed. Rows or columns that moved are followed in
//!   the graph, and a formula that no longer reads what it read is a seed.
//! * The scope is the seeds that are formulas, the formulas that read a seed,
//!   the formulas that read those, and so on, and the formulas that read what
//!   no write announces, such as the clock. An array in the scope brings in
//!   the formulas that read the area it fills.
//! * An array can come to fill an area it did not fill. That is known only
//!   once it has run, so the scope is then grown by the formulas that read
//!   the cells that changed hands, and evaluated again (`grow_scope`).
//!
//! Every formula is evaluated only when nothing is on record, the first time,
//! and when what formulas mean may have changed: sheets added, removed or
//! renamed, names, tables, settings. `Everything` lists the cases.

mod graph;
#[cfg(feature = "recalc_verify")]
mod verify;

use crate::constants::LAST_COLUMN;
use crate::evaluation::CellKey;
use crate::model::Model;
use crate::sheet_data::Write;
use crate::types::{
    ArrayKind, Cell, DefinedName, FormulaValue, MergedCell, Table, WorkbookSettings,
};

pub(crate) use graph::{Graph, Holds, Reads, Rect, Scope, Seeds, Shift};

/// How `Model::evaluate` recalculates.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RecalcMode {
    /// Every formula, every time.
    #[default]
    Full,
    /// Only the formulas an edit can change. Gives the same results as `Full`.
    Incremental,
    /// `Incremental`, followed by an evaluation of every formula that must
    /// find nothing to change. For tests.
    #[cfg(feature = "recalc_verify")]
    Verify,
}

impl RecalcMode {
    /// The mode a new model starts in. Under test it can be set from outside,
    /// `IRONCALC_RECALC=incremental cargo test`, so that every test of the
    /// crate runs incrementally.
    pub(crate) fn initial() -> Self {
        #[cfg(test)]
        match std::env::var("IRONCALC_RECALC").as_deref() {
            Ok("incremental") => return RecalcMode::Incremental,
            #[cfg(feature = "recalc_verify")]
            Ok("verify") => return RecalcMode::Verify,
            _ => {}
        }
        RecalcMode::Full
    }
}

/// Why an evaluation ran every formula.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Everything {
    /// The model recalculates in full.
    FullMode,
    /// Nothing is on record yet.
    FirstEvaluation,
    /// Sheets, names, tables or settings changed.
    WorkbookChanged,
}

/// What the last evaluation ran.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Ran {
    Everything(Everything),
    /// This many formulas.
    Only(usize),
}

/// Everything about a workbook that formulas can read and that neither the
/// cells nor the logs of writes tell. While it stays the same, the logs say
/// all that changed.
#[derive(PartialEq)]
struct Shape {
    /// The id and the name of every sheet, in order.
    sheets: Vec<(u32, String)>,
    defined_names: Vec<DefinedName>,
    tables: std::collections::HashMap<String, Table>,
    settings: WorkbookSettings,
}

impl Shape {
    fn of(model: &Model) -> Shape {
        let workbook = &model.workbook;
        Shape {
            sheets: workbook
                .worksheets
                .iter()
                .map(|worksheet| (worksheet.sheet_id, worksheet.name.clone()))
                .collect(),
            defined_names: workbook.defined_names.clone(),
            tables: workbook.tables.clone(),
            settings: workbook.settings.clone(),
        }
    }
}

/// What incremental recalculation keeps between evaluations.
#[derive(Default)]
pub(crate) struct Recalc {
    pub(crate) mode: RecalcMode,
    /// `None` until an evaluation of every formula has built it.
    graph: Option<Graph>,
    /// The shape of the workbook when the graph was last brought up to date.
    shape: Option<Shape>,
    /// The merged cells of every sheet at that time. A merged area blocks a
    /// spill, so merging and unmerging is a change to the cells of the area.
    merged: Vec<Vec<MergedCell>>,
    /// What each formula running now has read so far, innermost last. Empty
    /// unless reads are being put on record.
    frames: Vec<Frame>,
    /// Records no longer needed, kept for the room they hold: a formula that
    /// reads what it read before leaves nothing behind.
    spare: Vec<Reads>,
    tracing: bool,
    /// What the last evaluation ran.
    pub(crate) ran: Option<Ran>,
}

/// A formula that reads more single cells than this has them put on record
/// as the rectangle they lie in.
const MANY_CELLS: usize = 64;

/// What the formula running has read so far.
#[derive(Default)]
struct Frame {
    reads: Reads,
    /// The rectangle in `reads.rects` that stands for the single cells read
    /// once there were too many of them, if there is one.
    rest: Option<usize>,
}

impl Frame {
    fn cell(&mut self, cell: CellKey) {
        // A cell of a rectangle already on record is read as part of it.
        // Looking at the last few is enough: a walk reads the rectangles it
        // just recorded.
        let rects = &mut self.reads.rects;
        if rects.iter().rev().take(16).any(|rect| rect.contains(cell)) {
            return;
        }
        // A function that walks a range cell by cell, a million of them in a
        // whole column, must not leave a million cells on record. Past a few,
        // the cells of a sheet are kept as the rectangle around them: more
        // than was read, never less.
        if let Some(rest) = self.rest.filter(|rest| rects[*rest].sheet == cell.0) {
            rects[rest].include(cell);
            return;
        }
        self.reads.cells.push(cell);
        if self.reads.cells.len() > MANY_CELLS {
            let mut around = Rect::sized(cell, (1, 1));
            self.reads.cells.retain(|other| {
                if other.0 == cell.0 {
                    around.include(*other);
                }
                other.0 != cell.0
            });
            self.rest = Some(rects.len());
            rects.push(around);
        }
    }
}

impl Recalc {
    pub(crate) fn new(mode: RecalcMode) -> Recalc {
        Recalc {
            mode,
            ..Recalc::default()
        }
    }
}

impl Model<'_> {
    /// Sets how the model recalculates from now on. Results are the same in
    /// every mode; see `RecalcMode`.
    pub fn with_recalc_mode(mut self, mode: RecalcMode) -> Self {
        self.set_recalc_mode(mode);
        self
    }

    /// See `with_recalc_mode`.
    pub fn set_recalc_mode(&mut self, mode: RecalcMode) {
        self.recalc = Recalc::new(mode);
        self.record_writes(false);
    }

    /// A formula starts running: what it reads from now on is its own.
    pub(crate) fn begin_reads(&mut self) {
        if self.recalc.tracing {
            let mut reads = self.recalc.spare.pop().unwrap_or_default();
            reads.cells.clear();
            reads.rects.clear();
            reads.always = false;
            reads.positional = false;
            self.recalc.frames.push(Frame { reads, rest: None });
        }
    }

    /// The formula that began last stopped running. Returns what it read.
    pub(crate) fn end_reads(&mut self) -> Option<Reads> {
        self.recalc.frames.pop().map(|frame| frame.reads)
    }

    /// The formula that ran at `cell` stored a result computed from `reads`.
    pub(crate) fn commit_reads(&mut self, cell: CellKey, reads: Reads) {
        if let Some(graph) = &mut self.recalc.graph {
            if let Some(unchanged) = graph.set_reads(cell, reads) {
                self.recalc.spare.push(unchanged);
            }
        }
    }

    pub(crate) fn trace_cell(&mut self, cell: CellKey) {
        if let Some(frame) = self.recalc.frames.last_mut() {
            frame.cell(cell);
        }
    }

    pub(crate) fn trace_rect(&mut self, rect: Rect) {
        if rect.row1 > rect.row2 || rect.column1 > rect.column2 {
            return;
        }
        if let Some(frame) = self.recalc.frames.last_mut() {
            frame.reads.rects.push(rect);
        }
    }

    /// The formula running reads what no write announces: see `Reads::always`.
    pub(crate) fn trace_always(&mut self) {
        if let Some(frame) = self.recalc.frames.last_mut() {
            frame.reads.always = true;
        }
    }

    /// The formula running reads where cells are: see `Reads::positional`.
    pub(crate) fn trace_positional(&mut self) {
        if let Some(frame) = self.recalc.frames.last_mut() {
            frame.reads.positional = true;
        }
    }

    /// Starts or stops the logs of writes of every sheet, and returns what
    /// each held, with the index of its sheet.
    fn record_writes(&mut self, recording: bool) -> Vec<(u32, Vec<Write>)> {
        self.workbook
            .worksheets
            .iter_mut()
            .enumerate()
            .map(|(sheet, worksheet)| (sheet as u32, worksheet.sheet_data.writes.record(recording)))
            .collect()
    }

    /// Decides what the evaluation about to start runs: `None` for every
    /// formula, or the formulas the edits since the last evaluation can have
    /// changed. Stops the logs of writes: what the evaluation itself writes
    /// is not an edit.
    pub(crate) fn plan(&mut self) -> Option<Scope> {
        if self.recalc.mode == RecalcMode::Full {
            self.recalc.ran = Some(Ran::Everything(Everything::FullMode));
            return None;
        }
        let logs = self.record_writes(false);
        let shape = Shape::of(self);
        let everything = if self.recalc.graph.is_none() {
            Some(Everything::FirstEvaluation)
        } else if self.recalc.shape.as_ref() != Some(&shape) {
            Some(Everything::WorkbookChanged)
        } else {
            None
        };
        self.recalc.tracing = true;
        if let Some(why) = everything {
            self.recalc.graph = Some(Graph::default());
            self.recalc.shape = Some(shape);
            self.recalc.merged = self.merged_cells();
            self.recalc.ran = Some(Ran::Everything(why));
            return None;
        }
        let mut graph = self.recalc.graph.take()?;
        let mut seeds = Seeds::default();
        for (sheet, log) in logs {
            for write in log {
                match write {
                    Write::Cell(row, column) => seeds.cells.push((sheet, row, column)),
                    Write::Row(row) => self.seed_row(sheet, row, &graph, &mut seeds),
                    Write::Rows { first, delta } => {
                        let shift = Shift {
                            sheet,
                            rows: true,
                            first,
                            delta,
                        };
                        self.displace(&shift, &mut graph, &mut seeds);
                    }
                    Write::Columns { first, delta } => {
                        let shift = Shift {
                            sheet,
                            rows: false,
                            first,
                            delta,
                        };
                        self.displace(&shift, &mut graph, &mut seeds);
                    }
                }
            }
        }
        self.seed_merged_cells(&mut seeds);
        seeds.cells.sort_unstable();
        seeds.cells.dedup();
        for &position in &seeds.cells {
            if self.holds(position) == Holds::NoFormula {
                // Not a formula, or no longer one: it reads nothing.
                graph.remove(position);
                self.links.remove(&position);
            } else {
                seeds.formulas.push(position);
            }
        }
        seeds.formulas.extend(graph.always());
        let mut scope = Scope::default();
        graph.grow(&mut scope, seeds, |position| self.holds(position));
        self.recalc.graph = Some(graph);
        self.recalc.ran = Some(Ran::Only(scope.len()));
        Some(scope)
    }

    /// The evaluation ran the formulas of `scope`. `arrays` are the arrays
    /// among them and the cells each had filled before. Where an array now
    /// fills other cells, cells changed hands that the scope did not reckon
    /// with: the formulas that read them are added to it. Returns whether any
    /// was, and then the evaluation has to run again.
    pub(crate) fn grow_scope(
        &mut self,
        scope: &mut Scope,
        arrays: &[(CellKey, Vec<CellKey>)],
    ) -> bool {
        let mut seeds = Seeds::default();
        // An array of a fixed area (CSE) that had lost cells got them back.
        // Nothing restarts a pass for those: a formula of the scope that ran
        // before the array read them empty.
        let mut again = false;
        for (anchor, before) in arrays {
            let after = self.spilled_cells(*anchor);
            if *before != after {
                seeds
                    .cells
                    .extend(before.iter().filter(|cell| !after.contains(cell)));
                seeds
                    .cells
                    .extend(after.iter().filter(|cell| !before.contains(cell)));
                again |= self.array_kind(*anchor) == Some(ArrayKind::Cse);
            }
        }
        if seeds.cells.is_empty() {
            return false;
        }
        let Some(mut graph) = self.recalc.graph.take() else {
            return false;
        };
        let grew = graph.grow(scope, seeds, |position| self.holds(position));
        self.recalc.graph = Some(graph);
        if grew {
            self.recalc.ran = Some(Ran::Only(scope.len()));
        }
        grew || again
    }

    fn holds_error(&self, (sheet, row, column): CellKey) -> bool {
        let cell = self
            .workbook
            .worksheets
            .get(sheet as usize)
            .and_then(|worksheet| worksheet.sheet_data.cell(row, column));
        matches!(
            cell.as_deref(),
            Some(
                Cell::CellFormula {
                    v: FormulaValue::Error { .. },
                    ..
                } | Cell::ArrayFormula {
                    v: FormulaValue::Error { .. },
                    ..
                }
            )
        )
    }

    fn array_kind(&self, (sheet, row, column): CellKey) -> Option<ArrayKind> {
        let worksheet = self.workbook.worksheets.get(sheet as usize)?;
        match worksheet.sheet_data.cell(row, column).as_deref() {
            Some(Cell::ArrayFormula { kind, .. }) => Some(kind.clone()),
            _ => None,
        }
    }

    /// Whether the cell at a position is the anchor of an array, whatever
    /// the array fills now.
    pub(crate) fn is_array(&self, position: CellKey) -> bool {
        self.array_kind(position).is_some()
    }

    /// The cells the array at `anchor` fills now, the anchor left out, in
    /// natural order.
    pub(crate) fn spilled_cells(&self, anchor: CellKey) -> Vec<CellKey> {
        let Holds::Array(area) = self.holds(anchor) else {
            return Vec::new();
        };
        let (sheet, row, column) = anchor;
        let Some(worksheet) = self.workbook.worksheets.get(sheet as usize) else {
            return Vec::new();
        };
        let mut cells = Vec::new();
        for r in area.row1..=area.row2 {
            for (c, cell) in
                worksheet
                    .sheet_data
                    .cells_in_row_between(r, area.column1, area.column2)
            {
                if matches!(&*cell, Cell::SpillCell { a, .. } if *a == (row, column)) {
                    cells.push((sheet, r, c));
                }
            }
        }
        cells
    }

    /// Called when an evaluation is over, before conditional formatting.
    pub(crate) fn after_evaluation(&mut self) {
        if self.recalc.mode == RecalcMode::Full {
            return;
        }
        self.recalc.tracing = false;
        self.recalc.frames.clear();
        #[cfg(feature = "recalc_verify")]
        if self.recalc.mode == RecalcMode::Verify && matches!(self.recalc.ran, Some(Ran::Only(_))) {
            self.verify_against_everything();
        }
        self.record_writes(true);
    }

    /// What the cell at a position holds.
    pub(crate) fn holds(&self, position: CellKey) -> Holds {
        let (sheet, row, column) = position;
        let Some(worksheet) = self.workbook.worksheets.get(sheet as usize) else {
            return Holds::NoFormula;
        };
        match worksheet.sheet_data.cell(row, column).as_deref() {
            Some(Cell::CellFormula { .. }) => Holds::Formula,
            Some(Cell::ArrayFormula { r, .. }) if *r == (1, 1) => Holds::Formula,
            Some(Cell::ArrayFormula { r, .. }) => Holds::Array(Rect::sized(position, *r)),
            _ => Holds::NoFormula,
        }
    }

    /// The cells of a row were replaced. Anything in it may have changed, the
    /// formulas it holds are new, and the ones it held may be gone.
    fn seed_row(&self, sheet: u32, row: i32, graph: &Graph, seeds: &mut Seeds) {
        let rect = Rect {
            sheet,
            row1: row,
            column1: 1,
            row2: row,
            column2: LAST_COLUMN,
        };
        seeds.cells.extend(graph.formulas_in(&rect));
        if let Some(worksheet) = self.workbook.worksheets.get(sheet as usize) {
            let columns = worksheet.sheet_data.columns_in_row(row);
            seeds
                .cells
                .extend(columns.into_iter().map(|column| (sheet, row, column)));
        }
        seeds.rects.push(rect);
    }

    /// Rows or columns moved. What is kept by position follows them: the
    /// graph, the seeds found so far and the links formulas made. The formulas
    /// the move cut off from what they read are seeds. The order of the
    /// anchors does not follow: the anchors of the sheet were all reset, they
    /// are in the scope, and they find their place as they do when every
    /// formula runs.
    fn displace(&mut self, shift: &Shift, graph: &mut Graph, seeds: &mut Seeds) {
        seeds.cells = seeds
            .cells
            .iter()
            .filter_map(|cell| shift.cell(*cell))
            .collect();
        seeds.formulas = seeds
            .formulas
            .iter()
            .filter_map(|cell| shift.cell(*cell))
            .collect();
        // A rectangle that did not move whole stays: more is run than needed.
        for rect in &mut seeds.rects {
            if let Some(moved) = shift.rect(rect) {
                *rect = moved;
            }
        }
        let displaced = graph.displace(shift);
        seeds.formulas.extend(displaced.cut_off);
        // An error says in which cell it arose, by name. A formula that moved
        // with an error in it runs again to say it right.
        seeds.formulas.extend(
            displaced
                .moved
                .into_iter()
                .filter(|formula| self.holds_error(*formula)),
        );
        self.links = std::mem::take(&mut self.links)
            .into_iter()
            .filter_map(|(position, link)| Some((shift.cell(position)?, link)))
            .collect();
    }

    fn merged_cells(&self) -> Vec<Vec<MergedCell>> {
        self.workbook
            .worksheets
            .iter()
            .map(|worksheet| worksheet.merged_cells.clone())
            .collect()
    }

    /// Makes a seed of every area that was merged or unmerged.
    fn seed_merged_cells(&mut self, seeds: &mut Seeds) {
        let merged = self.merged_cells();
        for (sheet, (before, after)) in self.recalc.merged.iter().zip(&merged).enumerate() {
            let changed = before
                .iter()
                .filter(|area| !after.contains(area))
                .chain(after.iter().filter(|area| !before.contains(area)));
            for area in changed {
                seeds.rects.push(Rect::sized(
                    (sheet as u32, area.row, area.column),
                    (area.width, area.height),
                ));
            }
        }
        self.recalc.merged = merged;
    }
}
