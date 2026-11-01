//! Incremental recalculation: evaluating only the formulas an edit can change.
//!
//! The algorithm in short:
//!
//! * While a formula runs, the cells and the rectangles it reads are put on
//!   record. When it stores its result, they replace what it read the last
//!   time in the dependency graph (`graph.rs`).
//! * Every sheet logs the positions written to it (`sheet_data::Writes`). An
//!   evaluation takes the logs: the formulas that read a written position,
//!   the formulas that read those, and so on, are the *scope*, the formulas
//!   to run. So are written positions that hold a formula, and the formulas
//!   that read what no write announces, such as the clock.
//! * The pass is the ordinary one (`evaluation.rs`) with its roots limited to
//!   the scope. A formula outside the scope is not run: it holds what the
//!   last evaluation stored, and a formula reading it gets that.
//!
//! Everything else is left to a full evaluation, which also builds the graph
//! anew: the first evaluation; a change to anything but the contents of cells
//! (sheets, names, tables, hidden rows and columns, merged cells, settings);
//! inserting,
//! deleting or moving rows and columns; and a scope that touches an array
//! formula or the cells it spilled into, since what spills where is decided
//! by the order of a full pass. A full evaluation is what `RecalcMode::Full`
//! always runs, so falling back can make an evaluation slower, never different.
//!
//! A scope of more than half the formulas is evaluated in full as well: past
//! that, finding it costs about what it saves.

mod graph;
#[cfg(feature = "recalc_verify")]
mod verify;

use std::collections::HashSet;

use crate::evaluation::{CellKey, KeyBuild};
use crate::expressions::types::CellReferenceIndex;
use crate::model::Model;
use crate::types::{Cell, DefinedName, MergedCell, SheetState, Table, WorkbookSettings};

pub(crate) use graph::{Graph, Reads, Rect};

/// How `Model::evaluate` recalculates.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RecalcMode {
    /// Every formula, every time.
    #[default]
    Full,
    /// Only the formulas an edit can change. Gives the same results as `Full`.
    Incremental,
    /// `Incremental`, followed by a full evaluation that must find nothing to
    /// change. For tests.
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

/// Everything about a workbook that formulas can read besides the contents of
/// its cells. While it stays the same, the logs of writes say all that
/// changed.
#[derive(PartialEq)]
struct Shape {
    sheets: Vec<SheetShape>,
    defined_names: Vec<DefinedName>,
    tables: std::collections::HashMap<String, Table>,
    settings: WorkbookSettings,
}

#[derive(PartialEq)]
struct SheetShape {
    sheet_id: u32,
    name: String,
    state: SheetState,
    hidden_rows: Vec<i32>,
    hidden_columns: Vec<(i32, i32)>,
    /// A merged area blocks a spill.
    merged_cells: Vec<MergedCell>,
}

impl Shape {
    fn of(model: &Model) -> Shape {
        let workbook = &model.workbook;
        Shape {
            sheets: workbook
                .worksheets
                .iter()
                .map(|worksheet| SheetShape {
                    sheet_id: worksheet.sheet_id,
                    name: worksheet.name.clone(),
                    state: worksheet.state.clone(),
                    hidden_rows: worksheet
                        .rows
                        .iter()
                        .filter(|row| row.hidden)
                        .map(|row| row.r)
                        .collect(),
                    hidden_columns: worksheet
                        .cols
                        .iter()
                        .filter(|col| col.hidden)
                        .map(|col| (col.min, col.max))
                        .collect(),
                    merged_cells: worksheet.merged_cells.clone(),
                })
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
    /// `None` until a full evaluation in a mode that keeps it has built it.
    graph: Option<Graph>,
    /// The shape of the workbook when the graph was built.
    shape: Option<Shape>,
    /// The array anchors and the cells they spilled into, as the last full
    /// evaluation left them.
    arrays: HashSet<CellKey, KeyBuild>,
    /// What each formula running now has read so far, innermost last. Empty
    /// unless reads are being put on record.
    frames: Vec<Reads>,
    tracing: bool,
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
            self.recalc.frames.push(Reads::default());
        }
    }

    /// The formula that began last stopped running. Returns what it read.
    pub(crate) fn end_reads(&mut self) -> Option<Reads> {
        self.recalc.frames.pop()
    }

    /// The formula that ran at `cell` stored a result computed from `reads`.
    pub(crate) fn commit_reads(&mut self, cell: CellKey, reads: Reads) {
        if let Some(graph) = &mut self.recalc.graph {
            graph.set_reads(cell, reads);
        }
    }

    pub(crate) fn trace_cell(&mut self, cell: CellKey) {
        if let Some(frame) = self.recalc.frames.last_mut() {
            // A cell of a rectangle already on record is read as part of it.
            // Looking at the last few is enough: a walk reads the rectangle it
            // just recorded.
            if !frame
                .rects
                .iter()
                .rev()
                .take(4)
                .any(|rect| rect.contains(cell))
            {
                frame.cells.push(cell);
            }
        }
    }

    pub(crate) fn trace_rect(&mut self, rect: Rect) {
        if rect.row1 > rect.row2 || rect.column1 > rect.column2 {
            return;
        }
        if let Some(frame) = self.recalc.frames.last_mut() {
            frame.rects.push(rect);
        }
    }

    pub(crate) fn trace_beyond_cells(&mut self) {
        if let Some(frame) = self.recalc.frames.last_mut() {
            frame.beyond_cells = true;
        }
    }

    /// Starts or stops the logs of writes of every sheet, dropping what they hold.
    fn record_writes(&mut self, recording: bool) {
        for worksheet in &mut self.workbook.worksheets {
            worksheet.sheet_data.writes.record(recording);
        }
    }

    /// The positions written since the last evaluation, and whether rows or
    /// columns were moved. Stops the logs: what the evaluation itself writes
    /// is not an edit.
    fn take_writes(&mut self) -> (Vec<CellKey>, bool) {
        let mut cells = Vec::new();
        let mut structural = false;
        for (sheet, worksheet) in self.workbook.worksheets.iter_mut().enumerate() {
            let writes = &mut worksheet.sheet_data.writes;
            cells.extend(
                writes
                    .cells
                    .iter()
                    .map(|&(row, column)| (sheet as u32, row, column)),
            );
            structural |= writes.structural;
            writes.record(false);
        }
        cells.sort_unstable();
        cells.dedup();
        (cells, structural)
    }

    /// Evaluates the formulas the edits since the last evaluation can change,
    /// if that can be told, and returns `true`. Otherwise gets ready for the
    /// full evaluation that must follow, which builds the graph, and returns
    /// `false`.
    pub(crate) fn evaluate_incrementally(&mut self) -> bool {
        let (written, structural) = self.take_writes();
        let shape = Shape::of(self);
        let ready = !structural
            && self.recalc.graph.is_some()
            && self.recalc.shape.as_ref() == Some(&shape);
        if ready {
            if let Some(scope) = self.scope_of(&written) {
                let mut roots: Vec<CellReferenceIndex> = scope
                    .iter()
                    .map(|&(sheet, row, column)| CellReferenceIndex { sheet, row, column })
                    .collect();
                roots.sort_unstable_by_key(|cell| (cell.sheet, cell.row, cell.column));
                self.recalc.tracing = true;
                let completed = self.run_scoped_pass(roots, scope);
                self.recalc.tracing = false;
                if completed {
                    self.evaluate_conditional_formatting();
                    self.record_writes(true);
                    #[cfg(feature = "recalc_verify")]
                    if self.recalc.mode == RecalcMode::Verify {
                        self.verify_against_full();
                    }
                    return true;
                }
            }
        }
        self.recalc.graph = Some(Graph::default());
        self.recalc.shape = Some(shape);
        self.recalc.tracing = true;
        false
    }

    /// Called at the end of every full evaluation.
    pub(crate) fn after_full_evaluation(&mut self) {
        if self.recalc.mode == RecalcMode::Full {
            return;
        }
        self.recalc.tracing = false;
        self.recalc.frames.clear();
        self.recalc.arrays.clear();
        for (sheet, worksheet) in self.workbook.worksheets.iter().enumerate() {
            // Only a formula makes an array.
            if worksheet.shared_formulas.is_empty() {
                continue;
            }
            for (row, column, cell) in worksheet.sheet_data.cells() {
                if matches!(*cell, Cell::ArrayFormula { .. } | Cell::SpillCell { .. }) {
                    self.recalc.arrays.insert((sheet as u32, row, column));
                }
            }
        }
        self.record_writes(true);
    }

    /// The formulas to run after writes at `written`, or `None` if the
    /// evaluation must be full.
    fn scope_of(&mut self, written: &[CellKey]) -> Option<HashSet<CellKey, KeyBuild>> {
        let mut formulas = Vec::new();
        for &position in written {
            if self.recalc.arrays.contains(&position) {
                return None;
            }
            match self.cell_at(position) {
                Some(Cell::CellFormula { .. }) => formulas.push(position),
                Some(Cell::ArrayFormula { .. } | Cell::SpillCell { .. }) => return None,
                _ => {
                    // Not a formula, or no longer one: it reads nothing.
                    if let Some(graph) = &mut self.recalc.graph {
                        graph.remove(position);
                    }
                    self.links.remove(&position);
                }
            }
        }
        let graph = self.recalc.graph.as_mut()?;
        let limit = (graph.formula_count() / 2).max(1024);
        let mut scope = graph.scope(written, limit)?;
        scope.extend(formulas);
        if scope
            .iter()
            .any(|position| self.recalc.arrays.contains(position))
        {
            return None;
        }
        Some(scope)
    }

    fn cell_at(&self, (sheet, row, column): CellKey) -> Option<Cell> {
        let worksheet = self.workbook.worksheets.get(sheet as usize)?;
        worksheet
            .sheet_data
            .cell(row, column)
            .map(|cell| cell.into_owned())
    }
}
