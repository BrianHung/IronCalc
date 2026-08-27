#![deny(missing_docs)]

use std::collections::{HashMap, HashSet};
use std::vec::Vec;

use crate::dependency_graph::{Axis, DependencyGraph, Position, RecalcMode};

use crate::expressions::parser::static_analysis::run_static_analysis_on_node;
use crate::{
    calc_result::{CalcResult, Range},
    cell::CellValue,
    constants::{self, LAST_COLUMN, LAST_ROW},
    expressions::{
        lexer::LexerMode,
        parser::{
            move_formula::{move_formula, MoveContext},
            new_parser_english,
            static_analysis::StaticResult,
            stringify::{
                rename_defined_name_in_node, to_english_string, to_localized_string, to_rc_format,
            },
            ArrayNode, CompletionContext, NamedVariable, Node, Parser,
        },
        token::{get_error_by_name, Error, OpProduct, OpSum, OpUnary},
        types::*,
        utils::{self, is_valid_column_number, is_valid_identifier, is_valid_row},
    },
    formatter::{
        format::{format_number, parse_formatted_number},
        lexer::is_likely_date_number_format,
    },
    implicit_intersection::implicit_intersection,
    language::{get_default_language, get_language, Language},
    locale::{get_default_locale, get_locale, Locale},
    types::*,
    utils as common,
};

use crate::recalc::{Input, ReadSet, Write};
use crate::{cf_types::CfCellResult, tz::Tz};

mod array_index;
mod changed_cells;
pub(crate) mod cse_guard;
pub(crate) mod eval_ctx;
// `pub(crate)` for `EvalPass`: the benches report whether a pass stayed
// incremental or fell back, which is otherwise indistinguishable from outside.
pub(crate) mod incremental;
mod unstable_cells;
#[cfg(feature = "recalc_verify")]
mod verify;

pub(crate) use changed_cells::ChangedCells;
pub use changed_cells::ChangedSinceRead;

#[cfg(any(test, feature = "mock_time"))]
pub use crate::mock_time::get_milliseconds_since_epoch;

/// Number of milliseconds since January 1, 1970
/// Used by time and date functions. It takes the value from the environment:
/// * The Operative System
/// * The JavaScript environment
/// * Or mocked for tests
#[cfg(not(any(test, feature = "mock_time")))]
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::expect_used)]
pub fn get_milliseconds_since_epoch() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("problem with system time")
        .as_millis() as i64
}

/// Number of milliseconds since January 1, 1970
/// Used by time and date functions. It takes the value from the environment:
/// * The Operative System
/// * The JavaScript environment
/// * Or mocked for tests
#[cfg(not(any(test, feature = "mock_time")))]
#[cfg(target_arch = "wasm32")]
pub fn get_milliseconds_since_epoch() -> i64 {
    use js_sys::Date;
    Date::now() as i64
}

// The structure of a cell.
// It can be:
// * A single cell
// * The anchor of an array formula
// * The anchor of a dynamic formula
// * A part of an array formula spill
// * A part of a dynamic formula spill
pub(crate) enum CellStructure {
    SingleCell,
    ArrayFormula {
        range: (i32, i32),
    },
    DynamicFormula {
        range: (i32, i32),
    },
    SpillArray {
        anchor: (i32, i32),
        range: (i32, i32),
    },
    SpillDynamic {
        anchor: (i32, i32),
        range: (i32, i32),
    },
}

/// A CSE array rectangle: `(sheet, row, column, width, height)`.
type CseRect = (u32, i32, i32, i32, i32);

/// A cell might be evaluated or being evaluated
#[derive(Clone)]
pub(crate) enum CellState {
    /// The cell has already been evaluated
    Evaluated,
    /// The cell is being evaluated
    Evaluating,
}

/// A parsed formula for a defined name
#[derive(Clone)]
pub(crate) enum ParsedDefinedName {
    /// CellReference (`=C4`)
    CellReference(CellReferenceIndex),
    /// A Range (`=C4:D6`)
    RangeReference(Range),
    /// `=LAMBDA(params..., body)`
    LambdaDefinition(Vec<NamedVariable>, Node),
    /// `=SomethingElse`
    InvalidDefinedNameFormula,
}

/// Formatting settings for a locale
pub struct FmtSettings {
    /// Currency format
    pub currency: String,
    /// Currency format with symbol
    pub currency_format: String,
    /// Short date format
    pub short_date: String,
    /// Example of short date format
    pub short_date_example: String,
    /// Long date format
    pub long_date: String,
    /// Example of long date format
    pub long_date_example: String,
    /// Number format
    pub number_fmt: String,
    /// Example of number format
    pub number_example: String,
}

fn array_node_to_formula_value(node: ArrayNode) -> FormulaValue {
    match node {
        ArrayNode::Boolean(b) => FormulaValue::Boolean(b),
        ArrayNode::Number(n) => FormulaValue::Number(n),
        ArrayNode::String(s) => FormulaValue::Text(s),
        ArrayNode::Error(ei) => FormulaValue::Error {
            ei,
            o: String::new(),
            m: String::new(),
        },
        ArrayNode::Empty => FormulaValue::Number(0.0),
    }
}

fn array_node_to_spill_value(node: ArrayNode) -> SpillValue {
    match node {
        ArrayNode::Boolean(b) => SpillValue::Boolean(b),
        ArrayNode::Number(n) => SpillValue::Number(n),
        ArrayNode::String(s) => SpillValue::Text(s),
        ArrayNode::Error(ei) => SpillValue::Error(ei),
        ArrayNode::Empty => SpillValue::Number(0.0),
    }
}

fn formula_value_to_spill_value(v: &FormulaValue) -> SpillValue {
    match v {
        FormulaValue::Unevaluated => SpillValue::Error(Error::ERROR),
        FormulaValue::Boolean(b) => SpillValue::Boolean(*b),
        FormulaValue::Number(n) => SpillValue::Number(*n),
        FormulaValue::Text(s) => SpillValue::Text(s.clone()),
        FormulaValue::Error { ei, .. } => SpillValue::Error(ei.clone()),
    }
}

#[derive(Clone)]
pub(crate) enum CellOrRange {
    // (sheet, row, column)
    Cell((u32, i32, i32)),
    // (sheet, start_row, start_column, end_row, end_column)
    Range((u32, i32, i32, i32, i32)),
}

/// A dynamical IronCalc model.
///
/// Its is composed of a `Workbook`. Everything else are dynamical quantities:
///
/// * The Locale: a parsed version of the Workbook's locale
/// * The Timezone: an object representing the Workbook's timezone
/// * The language. Note that the timezone and the locale belong to the workbook while
///   the language can be different for different users looking _at the same_ workbook.
/// * Parsed Formulas: All the formulas in the workbook are parsed here (runtime only)
/// * A list of cells with its status (evaluating, evaluated, not evaluated)
/// * A dictionary with the shared strings and their indices.
///   This is an optimization for large files (~1 million rows)
pub struct Model<'a> {
    /// A Rust internal representation of an Excel workbook
    pub workbook: Workbook,
    /// A list of parsed formulas
    pub parsed_formulas: Vec<Vec<(Node, StaticResult)>>,
    /// A list of parsed defined names
    pub(crate) parsed_defined_names: HashMap<(Option<u32>, String), ParsedDefinedName>,
    /// An optimization to lookup strings faster
    pub(crate) shared_strings: HashMap<String, usize>,
    /// An instance of the parser
    pub(crate) parser: Parser<'a>,
    /// The list of cells with formulas that are evaluated or being evaluated
    pub(crate) cells: HashMap<(u32, i32, i32), CellState>,
    /// The locale of the model
    pub(crate) locale: &'a Locale,
    /// The language used
    pub(crate) language: &'a Language,
    /// The timezone used to evaluate the model
    pub(crate) tz: Tz,
    /// The view id. A view consists of a selected sheet and ranges.
    pub(crate) view_id: u32,
    /// A stack of variables used for LET function evaluation. The key is the variable id, and the value is the variable value.
    pub(crate) variable_stack: HashMap<usize, CalcResult>,
    /// Last variable id used. It is incremented every time a new variable is created (for example, when evaluating a LET function).
    pub(crate) last_variable_id: usize,
    /// Lambdas
    pub(crate) lambdas: HashMap<usize, (Vec<NamedVariable>, Node)>,
    /// Last lambda id used. It is incremented every time a new lambda is created.
    pub(crate) last_lambda_id: usize,
    /// The list of cells that might spill
    pub(crate) spill_cells: Vec<CellReferenceIndex>,
    /// A dictionary to keep track of which cells or ranges support a given cell.
    pub(crate) support: HashMap<CellReferenceIndex, Vec<CellOrRange>>,
    /// Evaluated CF results per cell, keyed by (sheet_index, row, column).
    /// Rebuilt from scratch on every call to evaluate_conditional_formatting().
    pub(crate) cf_cache: HashMap<(u32, i32, i32), Vec<CfCellResult>>,
    /// Dynamic links: links created by formulas like HYPERLINK
    pub(crate) links: HashMap<(u32, i32, i32), Link>,
    /// Forward dependency graph + dirty set backing incremental evaluation.
    pub(crate) graph: DependencyGraph,
    /// Which recalculation strategy `evaluate` uses (`Full` by default).
    pub(crate) recalc_mode: RecalcMode,
    /// When `Some`, `evaluate_cell` only recomputes cells in this set and returns
    /// the stored value for any cell outside it. Drives the incremental pass.
    pub(crate) recompute_scope: Option<HashSet<Position>>,
    /// Number of formula cells at the last full pass, maintained by the journal
    /// between passes. An incremental pass whose affected set approaches this
    /// recomputes about as much as a full pass but with extra bookkeeping, so it
    /// falls back to full instead.
    pub(crate) formula_cell_count: usize,
    /// Set by a structural edit: rows or columns of formula cells appeared or
    /// vanished without cell writes, so `formula_cell_count` must be recounted
    /// before the next fanout decision.
    pub(crate) formula_count_stale: bool,
    /// Lazily built list of CSE array rectangles `(sheet, row, column, width,
    /// height)`, used to reject writes into a member position even when a
    /// structural edit dropped the member cell itself. `None` means stale;
    /// structural edits, sheet changes, and CSE anchor writes reset it.
    pub(crate) cse_rects: Option<Vec<CseRect>>,
    /// Suspended while a structural rebuild -- a cell, row or column move --
    /// relocates cells through the user entry points; the member guard applies
    /// to user writes, not to the edit's own interim states. The flag inside is
    /// private to [`crate::model::cse_guard`]: only
    /// [`Model::with_cse_guard_suspended`] can flip it.
    pub(crate) cse_member_guard: cse_guard::CseMemberGuard,
    /// Set when an evaluation write changes an array footprint: a spill was
    /// written, a CSE range filled, or a dynamic anchor stored `#SPILL!`. The
    /// incremental pass that observes it falls back to Full, whose
    /// `collect_array_cells` rebuilds the array index exactly.
    pub(crate) wrote_array_cells: bool,
    /// Set whenever an evaluation re-enters a cell that is already evaluating,
    /// i.e. reports `#CIRC!`. Read (and reset) by the incremental scheduler.
    pub(crate) saw_circular_reference: bool,
    /// Stack of in-flight formula read sets. The evaluator pushes one per
    /// formula it is computing; nested `evaluate_cell` records on the top.
    pub(crate) read_stack: Vec<ReadSet>,
    /// What cells changed since the last [`Model::take_changed_cells`], backing
    /// the incremental delta API. See [`ChangedCells`].
    pub(crate) changed_cells: ChangedCells,
    /// Cell writes drained from the journal since the last incremental pass.
    /// These always-report in the delta; FormulaText/Hidden readers are dirty
    /// but only reported if their observable value moved.
    pub(crate) write_seeds: HashSet<Position>,
}

/// Whether `cell` belongs to the full pass's phase 1.
///
/// Phase 1 is every [`Cell::ArrayFormula`], CSE anchors included: a freshly set
/// anchor has placeholder members, and a reader of a member must not evaluate
/// before the anchor fills its rectangle, or the first pass stores a stale read
/// that only a second full pass would heal.
///
/// This is the single definition of phase-1 membership.
/// [`Model::collect_spill_cells`] selects phase 1 out of the whole workbook
/// with it; [`Model::in_full_pass_order`] reorders a cone with it. Those two
/// walk different inputs and so cannot share a traversal, but the order they
/// produce has to agree, and this is the half of it that can be shared.
pub(crate) fn is_phase_one_cell(cell: &Cell) -> bool {
    matches!(cell, Cell::ArrayFormula { .. })
}

// FIXME: Maybe this should be the same as CellReference
/// A struct pointing to a cell
pub struct CellIndex {
    /// Sheet index (0-indexed)
    pub index: u32,
    /// Row index
    pub row: i32,
    /// Column index
    pub column: i32,
}

impl<'a> Model<'a> {
    pub(crate) fn get_next_variable_id(&mut self) -> usize {
        let id = self.last_variable_id;
        self.last_variable_id += 1;
        id
    }
    fn clear_variable_stack(&mut self) {
        self.variable_stack.clear();
        self.last_variable_id = 0;
    }
    pub(crate) fn get_next_lambda_id(&mut self) -> usize {
        let id = self.last_lambda_id;
        self.last_lambda_id += 1;
        id
    }
    fn clear_lambdas(&mut self) {
        self.lambdas.clear();
        self.last_lambda_id = 0;
    }
    pub(crate) fn evaluate_node_with_reference(
        &mut self,
        node: &Node,
        cell: CellReferenceIndex,
    ) -> CalcResult {
        match node {
            Node::ReferenceKind {
                sheet_name: _,
                sheet_index,
                absolute_row,
                absolute_column,
                row,
                column,
            } => {
                let mut row1 = *row;
                let mut column1 = *column;
                if !absolute_row {
                    row1 += cell.row;
                }
                if !absolute_column {
                    column1 += cell.column;
                }
                CalcResult::Range {
                    left: CellReferenceIndex {
                        sheet: *sheet_index,
                        row: row1,
                        column: column1,
                    },
                    right: CellReferenceIndex {
                        sheet: *sheet_index,
                        row: row1,
                        column: column1,
                    },
                }
            }
            Node::RangeKind {
                sheet_name: _,
                sheet_index,
                absolute_row1,
                absolute_column1,
                row1,
                column1,
                absolute_row2,
                absolute_column2,
                row2,
                column2,
            } => {
                let mut row_left = *row1;
                let mut column_left = *column1;
                if !absolute_row1 {
                    row_left += cell.row;
                }
                if !absolute_column1 {
                    column_left += cell.column;
                }
                let mut row_right = *row2;
                let mut column_right = *column2;
                if !absolute_row2 {
                    row_right += cell.row;
                }
                if !absolute_column2 {
                    column_right += cell.column;
                }
                // FIXME: HACK. The parser is currently parsing Sheet3!A1:A10 as Sheet3!A1:(present sheet)!A10
                self.trace_rect(
                    *sheet_index,
                    row_left.min(row_right),
                    column_left.min(column_right),
                    row_left.max(row_right),
                    column_left.max(column_right),
                );
                CalcResult::Range {
                    left: CellReferenceIndex {
                        sheet: *sheet_index,
                        row: row_left,
                        column: column_left,
                    },
                    right: CellReferenceIndex {
                        sheet: *sheet_index,
                        row: row_right,
                        column: column_right,
                    },
                }
            }
            Node::ImplicitIntersection {
                automatic: _,
                child,
            } => match self.evaluate_node_with_reference(child, cell) {
                CalcResult::Range { left, right } => CalcResult::Range { left, right },
                _ => CalcResult::new_error(
                    Error::ERROR,
                    cell,
                    format!("Error with Implicit Intersection in cell {cell:?}"),
                ),
            },
            _ => self.evaluate_node_in_context(node, cell),
        }
    }

    fn get_range(&mut self, left: &Node, right: &Node, cell: CellReferenceIndex) -> CalcResult {
        let left_result = self.evaluate_node_with_reference(left, cell);
        let right_result = self.evaluate_node_with_reference(right, cell);
        match (left_result, right_result) {
            (
                CalcResult::Range {
                    left: left1,
                    right: right1,
                },
                CalcResult::Range {
                    left: left2,
                    right: right2,
                },
            ) => {
                if left1.row == right1.row
                    && left1.column == right1.column
                    && left2.row == right2.row
                    && left2.column == right2.column
                {
                    self.trace_rect(
                        left1.sheet,
                        left1.row.min(right2.row),
                        left1.column.min(right2.column),
                        left1.row.max(right2.row),
                        left1.column.max(right2.column),
                    );
                    return CalcResult::Range {
                        left: left1,
                        right: right2,
                    };
                }
                CalcResult::Error {
                    error: Error::VALUE,
                    origin: cell,
                    message: "Invalid range".to_string(),
                }
            }
            _ => CalcResult::Error {
                error: Error::VALUE,
                origin: cell,
                message: "Invalid range".to_string(),
            },
        }
    }

    pub(crate) fn formula_without_prefix<'b>(&self, value: &'b str) -> Option<&'b str> {
        if let Some(stripped) = value.strip_prefix('=') {
            if stripped.is_empty() {
                None
            } else {
                Some(stripped)
            }
        } else if let Some(stripped) = value.strip_prefix(['+', '-']) {
            if stripped.is_empty()
                || crate::cast::cast_number_with_locale(stripped, self.locale).is_some()
            {
                None
            } else {
                Some(value)
            }
        } else {
            None
        }
    }

    /// Parses a formula that is stored internally (always in English) and
    /// returns the resulting node.
    ///
    /// Formula strings kept outside of cells (defined names and conditional
    /// formatting rules) are always stored in English — see
    /// [Model::user_formula_to_internal]. They must therefore be parsed with
    /// the English language and locale regardless of the user's active
    /// language. This temporarily switches the parser, parses, and restores it.
    pub(crate) fn parse_internal_formula(&mut self, body: &str, context: &CellReferenceRC) -> Node {
        let locale = self.locale;
        let language = self.language;
        self.parser.set_locale(get_default_locale());
        self.parser.set_language(get_default_language());
        let node = self.parser.parse(body, context);
        self.parser.set_locale(locale);
        self.parser.set_language(language);
        node
    }

    /// Translates a formula the user typed (in the active language and locale)
    /// into the canonical English representation that is stored internally.
    ///
    /// The formula is first parsed in the active language/locale. If that fails
    /// it is parsed as English — this lets internally generated formulas (which
    /// are already English, e.g. produced by undo/redo or cut & paste) round
    /// trip unchanged regardless of the active language. Returns an error if the
    /// formula parses in neither. Any leading `=` is preserved.
    pub(crate) fn user_formula_to_internal(
        &mut self,
        formula: &str,
        context: &CellReferenceRC,
    ) -> Result<String, String> {
        let trimmed = formula.trim();
        let had_equals = trimmed.starts_with('=');
        let body = trimmed.strip_prefix('=').unwrap_or(trimmed);
        let mut node = self.parser.parse(body, context);
        if let Node::ParseErrorKind { .. } = node {
            // The user's language could not parse it: it might already be in the
            // internal English form.
            node = self.parse_internal_formula(body, context);
        }
        if let Node::ParseErrorKind { .. } = node {
            return Err(format!("Invalid formula: '{formula}'"));
        }
        let english = to_english_string(&node, context);
        Ok(if had_equals {
            format!("={english}")
        } else {
            english
        })
    }

    /// Returns completion information for a formula being edited in a cell.
    ///
    /// `formula` is the raw cell input (it may start with `=`) and `cursor` is a
    /// char offset into it. The references in the formula are resolved relative
    /// to the cell at (`sheet`, `row`, `column`). See
    /// [`CompletionContext`](crate::expressions::parser::CompletionContext).
    pub fn formula_completion(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        formula: &str,
        cursor: usize,
    ) -> Result<CompletionContext, String> {
        let sheet_name = self.workbook.worksheet(sheet)?.get_name();
        let cell_reference = CellReferenceRC {
            sheet: sheet_name,
            row,
            column,
        };
        // The parser works on the formula body, without the leading `=`. Drop it
        // and shift the cursor so it keeps pointing at the same character.
        let (body, cursor) = match formula.strip_prefix('=') {
            Some(rest) => (rest, cursor.saturating_sub(1)),
            None => (formula, cursor),
        };
        Ok(self.parser.parse_at_cursor(body, cursor, &cell_reference))
    }

    /// Translates an internally-stored (English) formula into the active
    /// language and locale for display to the user. Any leading `=` is
    /// preserved. If the formula fails to parse it is returned unchanged.
    pub(crate) fn internal_formula_to_display(
        &self,
        formula: &str,
        context: &CellReferenceRC,
    ) -> String {
        let trimmed = formula.trim();
        let had_equals = trimmed.starts_with('=');
        let body = trimmed.strip_prefix('=').unwrap_or(trimmed);
        if body.is_empty() {
            return formula.to_string();
        }
        // Stored formulas are in English, so parse with an English parser.
        let worksheet_names = self
            .workbook
            .worksheets
            .iter()
            .map(|s| s.get_name())
            .collect();
        let defined_names = self.workbook.get_defined_names_with_scope();
        let mut parser =
            new_parser_english(worksheet_names, defined_names, self.workbook.tables.clone());
        let node = parser.parse(body, context);
        if let Node::ParseErrorKind { .. } = node {
            return formula.to_string();
        }
        let local = to_localized_string(&node, context, self.locale, self.language);
        if had_equals {
            format!("={local}")
        } else {
            local
        }
    }

    /// Evaluates a formula string on a sheet, returning the numeric result.
    /// Assumes the workbook has already been evaluated (cell values are up-to-date).
    /// Returns `None` if the formula is invalid or does not produce a number.
    pub(crate) fn evaluate_formula(&mut self, formula: &str, sheet: u32) -> Option<f64> {
        let body = formula.trim().strip_prefix('=').unwrap_or(formula.trim());
        if body.is_empty() {
            return None;
        }
        let sheet_name = self.workbook.worksheets.get(sheet as usize)?.get_name();
        let context_rc = CellReferenceRC {
            sheet: sheet_name,
            row: 1,
            column: 1,
        };
        let node = self.parse_internal_formula(body, &context_rc);
        let context_index = CellReferenceIndex {
            sheet,
            row: 1,
            column: 1,
        };
        match self.evaluate_node_in_context(&node, context_index) {
            CalcResult::Number(n) => Some(n),
            _ => None,
        }
    }

    pub(crate) fn evaluate_node_in_context(
        &mut self,
        node: &Node,
        cell: CellReferenceIndex,
    ) -> CalcResult {
        use Node::*;
        match node {
            OpSumKind { kind, left, right } => match kind {
                OpSum::Add => self
                    .eval_ctx()
                    .handle_arithmetic(left, right, cell, &|f1, f2| Ok(f1 + f2)),
                OpSum::Minus => self
                    .eval_ctx()
                    .handle_arithmetic(left, right, cell, &|f1, f2| Ok(f1 - f2)),
            },
            NumberKind(value) => CalcResult::Number(*value),
            StringKind(value) => CalcResult::String(value.replace(r#""""#, r#"""#)),
            BooleanKind(value) => CalcResult::Boolean(*value),
            ReferenceKind {
                sheet_name: _,
                sheet_index,
                absolute_row,
                absolute_column,
                row,
                column,
            } => {
                let mut row1 = *row;
                let mut column1 = *column;
                if !absolute_row {
                    row1 += cell.row;
                }
                if !absolute_column {
                    column1 += cell.column;
                }
                self.support
                    .entry(cell)
                    .or_default()
                    .push(CellOrRange::Cell((*sheet_index, row1, column1)));
                self.evaluate_cell(CellReferenceIndex {
                    sheet: *sheet_index,
                    row: row1,
                    column: column1,
                })
            }
            WrongReferenceKind { .. } => {
                CalcResult::new_error(Error::REF, cell, "Wrong reference".to_string())
            }
            OpRangeKind { left, right } => self.get_range(left, right, cell),
            WrongRangeKind { .. } => {
                CalcResult::new_error(Error::REF, cell, "Wrong range".to_string())
            }
            RangeKind {
                sheet_index,
                row1,
                column1,
                row2,
                column2,
                absolute_column1,
                absolute_row2,
                absolute_row1,
                absolute_column2,
                sheet_name: _,
            } => {
                let r1 = if *absolute_row1 {
                    *row1
                } else {
                    *row1 + cell.row
                };
                let r2 = if *absolute_row2 {
                    *row2
                } else {
                    *row2 + cell.row
                };
                let c1 = if *absolute_column1 {
                    *column1
                } else {
                    *column1 + cell.column
                };
                let c2 = if *absolute_column2 {
                    *column2
                } else {
                    *column2 + cell.column
                };
                self.support
                    .entry(cell)
                    .or_default()
                    .push(CellOrRange::Range((
                        *sheet_index,
                        r1.min(r2),
                        c1.min(c2),
                        r1.max(r2),
                        c1.max(c2),
                    )));
                self.trace_rect(*sheet_index, r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2));
                CalcResult::Range {
                    left: CellReferenceIndex {
                        sheet: *sheet_index,
                        row: r1.min(r2),
                        column: c1.min(c2),
                    },
                    right: CellReferenceIndex {
                        sheet: *sheet_index,
                        row: r1.max(r2),
                        column: c1.max(c2),
                    },
                }
            }
            OpConcatenateKind { left, right } => {
                self.eval_ctx().handle_concatenate(left, right, cell)
            }
            OpProductKind { kind, left, right } => match kind {
                OpProduct::Times => {
                    self.eval_ctx()
                        .handle_arithmetic(left, right, cell, &|f1, f2| Ok(f1 * f2))
                }
                OpProduct::Divide => {
                    self.eval_ctx()
                        .handle_arithmetic(left, right, cell, &|f1, f2| {
                            if f2 == 0.0 {
                                Err(Error::DIV)
                            } else {
                                Ok(f1 / f2)
                            }
                        })
                }
            },
            OpPowerKind { left, right } => {
                self.eval_ctx()
                    .handle_arithmetic(left, right, cell, &|f1, f2| Ok(f1.powf(f2)))
            }
            FunctionKind { kind, args } => self.eval_ctx().evaluate_function(kind, args, cell),
            NamedFunctionKind { name, args, id } => {
                let lambda_result = if let Some(var_id) = id {
                    // Bound by LET — look up the variable, which should be a Lambda.
                    match self.variable_stack.get(&(*var_id as usize)) {
                        Some(v) => v.clone(),
                        None => {
                            return CalcResult::new_error(
                                Error::NAME,
                                cell,
                                format!("Variable \"{name}\" not found in scope."),
                            )
                        }
                    }
                } else {
                    // Not bound by LET — look up as a defined-name Lambda.
                    // Prefer sheet-local (current sheet) over global (scope = None),
                    // matching Excel's name resolution order.
                    let name_lower = name.to_lowercase();
                    let found = self
                        .parsed_defined_names
                        .get(&(Some(cell.sheet), name_lower.clone()))
                        .or_else(|| self.parsed_defined_names.get(&(None, name_lower)))
                        .cloned();
                    match found {
                        Some(ParsedDefinedName::LambdaDefinition(param_names, body)) => {
                            let lambda_id = self.get_next_lambda_id();
                            self.lambdas.insert(lambda_id, (param_names, body));
                            CalcResult::Lambda(lambda_id)
                        }
                        _ => {
                            return CalcResult::new_error(
                                Error::NAME,
                                cell,
                                format!("Invalid function: {name}"),
                            )
                        }
                    }
                };
                self.eval_ctx().call_lambda(lambda_result, args, cell)
            }
            ArrayKind(s) => CalcResult::Array(s.to_owned()),
            DefinedNameKind((name, scope, _)) => {
                self.trace_input(Input::Name {
                    name: name.clone(),
                    scope: *scope,
                });
                if let Ok(Some(parsed_defined_name)) = self.get_parsed_defined_name(name, *scope) {
                    match parsed_defined_name {
                        ParsedDefinedName::RangeReference(range) => {
                            self.trace_rect(
                                range.left.sheet,
                                range.left.row,
                                range.left.column,
                                range.right.row,
                                range.right.column,
                            );
                            CalcResult::Range {
                                left: range.left,
                                right: range.right,
                            }
                        }
                        ParsedDefinedName::CellReference(reference) => {
                            self.evaluate_cell(reference)
                        }
                        ParsedDefinedName::LambdaDefinition(param_names, body) => {
                            let lambda_id = self.get_next_lambda_id();
                            self.lambdas.insert(lambda_id, (param_names, body));
                            CalcResult::Lambda(lambda_id)
                        }
                        ParsedDefinedName::InvalidDefinedNameFormula => CalcResult::new_error(
                            Error::NAME,
                            cell,
                            format!("Defined name \"{name}\" is not a reference."),
                        ),
                    }
                } else {
                    CalcResult::new_error(
                        Error::NAME,
                        cell,
                        format!("Defined name \"{name}\" not found."),
                    )
                }
            }
            TableNameKind(s) => CalcResult::new_error(
                Error::NAME,
                cell,
                format!("table name \"{s}\" not supported."),
            ),
            NamedVariableKind { name, id: Some(id) } => {
                match self.variable_stack.get(&(*id as usize)) {
                    Some(v) => v.clone(),
                    None => CalcResult::new_error(
                        Error::NAME,
                        cell,
                        format!("Variable \"{name}\" not found in scope."),
                    ),
                }
            }
            NamedVariableKind { name, id: None } => CalcResult::new_error(
                Error::NAME,
                cell,
                format!("Variable name \"{name}\" not found."),
            ),
            CompareKind { kind, left, right } => {
                self.eval_ctx().handle_comparison(left, right, cell, kind)
            }
            UnaryKind { kind, right } => {
                let r = match self.eval_ctx().get_number(right, cell) {
                    Ok(f) => f,
                    Err(s) => {
                        return s;
                    }
                };
                match kind {
                    OpUnary::Minus => CalcResult::Number(-r),
                    OpUnary::Percentage => CalcResult::Number(r / 100.0),
                }
            }
            ErrorKind(kind) => CalcResult::new_error(kind.clone(), cell, "".to_string()),
            ParseErrorKind {
                formula, message, ..
            } => CalcResult::new_error(
                Error::ERROR,
                cell,
                format!("Error parsing {formula}: {message}"),
            ),
            EmptyArgKind => CalcResult::EmptyArg,
            SpillRangeOperator { child } => match self.evaluate_node_with_reference(child, cell) {
                CalcResult::Range { left, right } => {
                    if left != right {
                        return CalcResult::new_error(
                            Error::ERROR,
                            cell,
                            format!("Error with Spill Range Operator in cell {cell:?}"),
                        );
                    }
                    // The `#` operator reads the anchor's spill geometry. Evaluate
                    // (and therefore trace) the anchor even when it is empty, so a
                    // later write there re-runs this formula (`=E15#` after E15 is
                    // filled, COUNT(F15#) after F15 is filled).
                    let _ = self.evaluate_cell(left);
                    let sheet = left.sheet;
                    let row = left.row;
                    let column = left.column;
                    let worksheet = match self.workbook.worksheet(sheet) {
                        Ok(s) => s,
                        Err(e) => {
                            return CalcResult::new_error(
                                Error::REF,
                                cell,
                                format!("Sheet index {sheet} not found: {e}"),
                            );
                        }
                    };
                    match worksheet.get_cell_spill(row, column) {
                        Ok((width, height)) => CalcResult::Range {
                            left: CellReferenceIndex { sheet, row, column },
                            right: CellReferenceIndex {
                                sheet,
                                row: row + height - 1,
                                column: column + width - 1,
                            },
                        },
                        Err(e) => CalcResult::new_error(
                            Error::REF,
                            cell,
                            format!("Cell {sheet}!{row},{column} not found: {e}"),
                        ),
                    }
                }
                _ => CalcResult::new_error(
                    Error::ERROR,
                    cell,
                    format!("Error with Spill Range Operator in cell {cell:?}"),
                ),
            },
            ImplicitIntersection {
                automatic: _,
                child,
            } => match self.evaluate_node_with_reference(child, cell) {
                CalcResult::Range { left, right } => {
                    match implicit_intersection(&cell, &Range { left, right }) {
                        Some(cell_reference) => self.evaluate_cell(cell_reference),
                        None => CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            format!("Error with Implicit Intersection in cell {cell:?}"),
                        ),
                    }
                }
                _ => self.evaluate_node_in_context(child, cell),
            },
            LambdaDefKind { parameters, body } => {
                let id = self.get_next_lambda_id();
                self.lambdas.insert(id, (parameters.clone(), *body.clone()));
                CalcResult::Lambda(id)
            }
            LambdaCallKind { lambda, args } => {
                let lambda_result = self.evaluate_node_in_context(lambda, cell);
                self.eval_ctx().call_lambda(lambda_result, args, cell)
            }
        }
    }

    fn cell_reference_to_string(
        &self,
        cell_reference: &CellReferenceIndex,
    ) -> Result<String, String> {
        let sheet = self.workbook.worksheet(cell_reference.sheet)?;
        let column = utils::number_to_column(cell_reference.column)
            .ok_or_else(|| "Invalid column".to_string())?;
        if !is_valid_row(cell_reference.row) {
            return Err("Invalid row".to_string());
        }
        Ok(format!("{}!{}{}", sheet.name, column, cell_reference.row))
    }

    fn get_value_from_array(
        &self,
        array: &[Vec<ArrayNode>],
        row: i32,
        column: i32,
    ) -> Option<ArrayNode> {
        let width = array[0].len() as i32;
        let height = array.len() as i32;
        if row < 1 || row > height || column < 1 || column > width {
            return None;
        }
        let value = &array[(row - 1) as usize][(column - 1) as usize];
        Some(value.clone())
    }

    /// Sets `result` in the cell given by `sheet` sheet index, row and column
    /// Note that will panic if the cell does not exist
    /// It will do nothing if the cell does not have a formula
    /// If the result is an array it will spill over other cells
    /// If the formula is an array formula it will update the spill area.
    ///    If the array is smaller than the spill area it will fill the remaining cells with #N/A error
    ///    If the array is just one element it will fill the original range with that element
    fn set_cells_with_result(
        &mut self,
        cell_reference: CellReferenceIndex,
        cell: &Cell,
        result: &CalcResult,
    ) -> Result<(), String> {
        let CellReferenceIndex { sheet, column, row } = cell_reference;
        let original_range = match cell {
            Cell::ArrayFormula {
                r,
                kind: ArrayKind::Cse,
                ..
            } => Some((false, (r.0, r.1))),
            Cell::ArrayFormula {
                r,
                kind: ArrayKind::Dynamic,
                ..
            } => Some((true, (r.0, r.1))),
            _ => None,
        };
        let s = cell.get_style();
        let formula = match cell.get_formula() {
            Some(f) => f,
            None => return Ok(()),
        };
        // Handle array results separately: they always return early, writing all cells
        // themselves. By dispatching here we avoid needing an unreachable arm in the
        // `new_cell` match below.
        if let CalcResult::Array(array) = result {
            if array.is_empty() || array[0].is_empty() {
                return self.set_cells_with_result(
                    cell_reference,
                    cell,
                    &CalcResult::new_error(
                        Error::CALC,
                        cell_reference,
                        "Formula produced a zero-size array".to_string(),
                    ),
                );
            }
            let array_width = array[0].len() as i32;
            let array_height = array.len() as i32;

            match original_range {
                Some((true, _)) => {
                    if row + array_height - 1 > LAST_ROW || column + array_width - 1 > LAST_COLUMN {
                        return self.set_cells_with_result(
                            cell_reference,
                            cell,
                            &CalcResult::new_error(
                                Error::SPILL,
                                cell_reference,
                                "Spill would exceed worksheet bounds".to_string(),
                            ),
                        );
                    }
                    // Check that the full spill area (based on actual result dimensions) is clear.
                    // The stored range may be (1,1) on first evaluation, so we must re-check here.
                    let sheet_data = &self.workbook.worksheets[sheet as usize].sheet_data;
                    for r in row..row + array_height {
                        let row_data = sheet_data.get(&r);
                        for c in column..column + array_width {
                            if r == row && c == column {
                                continue;
                            }
                            // A cell blocks spilling only if it is occupied by something
                            // other than an empty cell or a spill cell that already belongs
                            // to this formula.  Own spill cells are about to be overwritten
                            // and must never prevent the formula from re-spilling (this
                            // matters after undo restores a SpillCell while the anchor's
                            // stored `r` is still (1,1) from a prior #SPILL! evaluation).
                            let blocking = row_data
                                .and_then(|row_map| row_map.get(&c))
                                .map(|cell| match cell {
                                    Cell::EmptyCell { .. } => false,
                                    Cell::SpillCell { a, .. } if *a == (row, column) => false,
                                    _ => true,
                                })
                                .unwrap_or(false);
                            if blocking {
                                self.trace_cell(sheet, r, c);
                                return self.set_cells_with_result(
                                    cell_reference,
                                    cell,
                                    &CalcResult::new_error(
                                        Error::SPILL,
                                        cell_reference,
                                        "Cannot spill array result".to_string(),
                                    ),
                                );
                            }
                        }
                    }
                    if array_width != 1 || array_height != 1 {
                        // Spill cells are about to be written: the array
                        // footprint changed, and the array index only rebuilds
                        // on a full pass.
                        self.wrote_array_cells = true;
                    }
                    let worksheet = &mut self.workbook.worksheets[sheet as usize];
                    // Dynamic formula: spill the array into adjacent cells.
                    // Cells are created on demand via update_cell since they may not exist yet.
                    for r in row..row + array_height {
                        for c in column..column + array_width {
                            let value = array[(r - row) as usize][(c - column) as usize].clone();
                            let cell = if r == row && c == column {
                                Cell::ArrayFormula {
                                    f: formula,
                                    s,
                                    r: (array_width, array_height),
                                    kind: ArrayKind::Dynamic,
                                    v: array_node_to_formula_value(value),
                                }
                            } else {
                                let existing_style = worksheet.get_style(r, c);
                                Cell::SpillCell {
                                    a: (row, column),
                                    s: existing_style,
                                    v: array_node_to_spill_value(value),
                                }
                            };
                            worksheet.update_cell(r, c, cell)?;
                        }
                    }
                    return Ok(());
                }
                Some((false, (original_width, original_height))) => {
                    // CSE array formula: fill the declared range with the array values.
                    // Use relative indices for get_value_from_array (1-based).
                    // Cells are created on demand via update_cell: a structural edit
                    // can drop a member of the declared range, and the next full
                    // pass must refill it rather than error on the missing slot.
                    for r in row..row + original_height {
                        for c in column..column + original_width {
                            let rel_row = r - row + 1;
                            let rel_col = c - column + 1;
                            let value = self.get_value_from_array(array, rel_row, rel_col);
                            let new_cell = if r == row && c == column {
                                let fv = match value {
                                    Some(node) => array_node_to_formula_value(node),
                                    None => FormulaValue::Error {
                                        ei: Error::NIMPL,
                                        o: "".to_string(),
                                        m: "Unexpected array result".to_string(),
                                    },
                                };
                                Cell::ArrayFormula {
                                    f: formula,
                                    s,
                                    r: (original_width, original_height),
                                    kind: ArrayKind::Cse,
                                    v: fv,
                                }
                            } else {
                                let sv = match value {
                                    Some(node) => array_node_to_spill_value(node),
                                    None => SpillValue::Error(Error::VALUE),
                                };
                                let existing_style =
                                    self.workbook.worksheets[sheet as usize].get_style(r, c);
                                Cell::SpillCell {
                                    s: existing_style,
                                    a: (row, column),
                                    v: sv,
                                }
                            };
                            let worksheet = &mut self.workbook.worksheets[sheet as usize];
                            worksheet.update_cell(r, c, new_cell)?;
                        }
                    }
                    // All cells (anchor + spills) have been written above.
                    self.wrote_array_cells = true;
                    return Ok(());
                }
                None => {
                    // Scalar formula produced a computed array at runtime. A 1x1
                    // array is a single value just wrapped; unwrap it. A larger
                    // array has no source coordinates to intersect against, so it
                    // takes its top-left element, the legacy behavior for
                    // positionless arrays. Ranges never reach here: evaluate_cell
                    // applies implicit intersection while their coordinates are
                    // still known.
                    let coerced = match self.get_value_from_array(array, 1, 1) {
                        Some(node) => array_node_to_formula_value(node),
                        None => FormulaValue::Error {
                            ei: Error::VALUE,
                            o: "".to_string(),
                            m: "Unexpected array result".to_string(),
                        },
                    };
                    *self.workbook.worksheets[sheet as usize]
                        .sheet_data
                        .get_mut(&row)
                        .ok_or("expected a row")?
                        .get_mut(&column)
                        .ok_or("expected a column")? = Cell::CellFormula {
                        f: formula,
                        s,
                        v: coerced,
                    };
                    return Ok(());
                }
            }
        }

        let formula_value = match result {
            CalcResult::Number(value) => {
                // safety belt
                if value.is_nan() || value.is_infinite() {
                    // This should never happen, is there a way we can log this events?
                    return self.set_cells_with_result(
                        cell_reference,
                        cell,
                        &CalcResult::Error {
                            error: Error::NUM,
                            origin: cell_reference,
                            message: "".to_string(),
                        },
                    );
                }
                FormulaValue::Number(*value)
            }
            CalcResult::String(value) => FormulaValue::Text(value.clone()),
            CalcResult::Boolean(value) => FormulaValue::Boolean(*value),
            CalcResult::Error {
                error,
                origin,
                message,
            } => {
                let o = match self.cell_reference_to_string(origin) {
                    Ok(s) => s,
                    Err(_) => "".to_string(),
                };
                FormulaValue::Error {
                    ei: error.clone(),
                    o,
                    m: message.to_string(),
                }
            }
            CalcResult::Range { .. } => {
                // This should never happen
                debug_assert!(false, "Unexpected range result in non-array formula");
                return Err("Cannot set a range as cell value".to_string());
            }
            CalcResult::EmptyCell | CalcResult::EmptyArg => {
                // Excel coerces a blank formula result to 0 (cached as
                // `<v>0</v>`); storing 0 keeps same-pass readers and later
                // out-of-cone readers in agreement, so Full is order-independent.
                FormulaValue::Number(0.0)
            }
            // CalcResult::Array is handled before this match (see above); it always returns early.
            CalcResult::Array(_) | CalcResult::Lambda(_) => {
                debug_assert!(false, "Unexpected array result in non-array formula");
                return Err("Unexpected array result in non-array formula".to_string());
            }
        };

        let new_cell = match original_range {
            Some((is_dynamic, (width, height))) => {
                let (kind, r) = if is_dynamic {
                    (ArrayKind::Dynamic, (1, 1))
                } else {
                    (ArrayKind::Cse, (width, height))
                };
                if matches!(
                    formula_value,
                    FormulaValue::Error {
                        ei: Error::SPILL,
                        ..
                    }
                ) {
                    // A blocked anchor belongs in the array index (full-mode
                    // same-pass readers see the live array, not the stored
                    // error), and the index only rebuilds on a full pass.
                    self.wrote_array_cells = true;
                }
                Cell::ArrayFormula {
                    f: formula,
                    s,
                    r,
                    kind,
                    v: formula_value.clone(),
                }
            }
            None => Cell::CellFormula {
                f: formula,
                s,
                v: formula_value.clone(),
            },
        };

        // If the cell is the anchor of a CSE array formula, fill all spill cells
        if let Some((false, (width, height))) = original_range {
            let spill_value = formula_value_to_spill_value(&formula_value);
            let ws = &mut self.workbook.worksheets[sheet as usize];
            for r in row..row + height {
                for c in column..column + width {
                    if r == row && c == column {
                        continue;
                    }
                    let existing_style = ws.get_style(r, c);
                    ws.update_cell(
                        r,
                        c,
                        Cell::SpillCell {
                            a: (row, column),
                            s: existing_style,
                            v: spill_value.clone(),
                        },
                    )?;
                }
            }
        }

        self.workbook.worksheets[sheet as usize].update_cell(row, column, new_cell)?;
        Ok(())
    }

    /// Sets the color of the sheet tab.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::{Model, types::Color};
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// assert_eq!(model.workbook.worksheet(0)?.color, Color::None);
    /// model.set_sheet_color(0, &Color::Rgb("#DBBE29".to_string()))?;
    /// assert_eq!(model.workbook.worksheet(0)?.color, Color::Rgb("#DBBE29".to_string()));
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_sheet_color(&mut self, sheet: u32, color: &Color) -> Result<(), String> {
        let worksheet = self.workbook.worksheet_mut(sheet)?;
        worksheet.color = color.clone();
        Ok(())
    }

    /// Changes the visibility of a sheet
    pub fn set_sheet_state(&mut self, sheet: u32, state: SheetState) -> Result<(), String> {
        let worksheet = self.workbook.worksheet_mut(sheet)?;
        worksheet.state = state;
        Ok(())
    }

    /// Sets the workbook theme.
    pub fn set_theme(&mut self, theme: crate::types::Theme) {
        self.workbook.theme = theme;
        self.evaluate_conditional_formatting();
    }

    /// Returns the Theme
    pub fn get_theme(&self) -> Theme {
        self.workbook.theme.clone()
    }

    /// Makes the grid lines in the sheet visible (`true`) or hidden (`false`)
    pub fn set_show_grid_lines(&mut self, sheet: u32, show_grid_lines: bool) -> Result<(), String> {
        let worksheet = self.workbook.worksheet_mut(sheet)?;
        worksheet.show_grid_lines = show_grid_lines;
        Ok(())
    }

    // Returns the 'single' value of a cell. Not arrays or ranges.
    fn get_cell_value(&self, cell: &Cell, cell_reference: CellReferenceIndex) -> CalcResult {
        use Cell::*;
        match cell {
            EmptyCell { .. } => CalcResult::EmptyCell,
            BooleanCell { v, .. } => CalcResult::Boolean(*v),
            NumberCell { v, .. } => CalcResult::Number(*v),
            ErrorCell { ei, .. } => {
                let message = ei.to_localized_error_string(self.language);
                CalcResult::new_error(ei.clone(), cell_reference, message)
            }
            SharedString { si, .. } => {
                if let Some(s) = self.workbook.shared_strings.get(*si as usize) {
                    CalcResult::String(s.clone())
                } else {
                    let message = "Invalid shared string".to_string();
                    CalcResult::new_error(Error::ERROR, cell_reference, message)
                }
            }
            CellFormula {
                v: FormulaValue::Unevaluated,
                ..
            }
            | ArrayFormula {
                v: FormulaValue::Unevaluated,
                ..
            } => CalcResult::Error {
                error: Error::ERROR,
                origin: cell_reference,
                message: "Unevaluated formula".to_string(),
            },
            CellFormula {
                v: FormulaValue::Boolean(v),
                ..
            }
            | ArrayFormula {
                v: FormulaValue::Boolean(v),
                ..
            } => CalcResult::Boolean(*v),
            CellFormula {
                v: FormulaValue::Number(v),
                ..
            }
            | ArrayFormula {
                v: FormulaValue::Number(v),
                ..
            } => CalcResult::Number(*v),
            CellFormula {
                v: FormulaValue::Text(v),
                ..
            }
            | ArrayFormula {
                v: FormulaValue::Text(v),
                ..
            } => CalcResult::String(v.clone()),
            CellFormula {
                v: FormulaValue::Error { ei, o, m },
                ..
            }
            | ArrayFormula {
                v: FormulaValue::Error { ei, o, m },
                ..
            } => {
                if let Some(cell_reference) = self.parse_reference(o) {
                    CalcResult::new_error(ei.clone(), cell_reference, m.clone())
                } else {
                    CalcResult::Error {
                        error: ei.clone(),
                        origin: cell_reference,
                        message: ei.to_localized_error_string(self.language),
                    }
                }
            }
            SpillCell {
                v: SpillValue::Number(v),
                ..
            } => CalcResult::Number(*v),
            SpillCell {
                v: SpillValue::Boolean(v),
                ..
            } => CalcResult::Boolean(*v),
            SpillCell {
                v: SpillValue::Text(v),
                ..
            } => CalcResult::String(v.clone()),
            SpillCell {
                v: SpillValue::Error(ei),
                ..
            } => {
                let message = ei.to_localized_error_string(self.language);
                CalcResult::new_error(ei.clone(), cell_reference, message)
            }
        }
    }

    /// Returns `true` if the cell is completely empty.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// assert_eq!(model.is_empty_cell(0, 1, 1)?, true);
    /// model.set_user_input(0, 1, 1, "Attention is all you need".to_string());
    /// assert_eq!(model.is_empty_cell(0, 1, 1)?, false);
    /// # Ok(())
    /// # }
    /// ```
    pub fn is_empty_cell(&self, sheet: u32, row: i32, column: i32) -> Result<bool, String> {
        self.workbook.worksheet(sheet)?.is_empty_cell(row, column)
    }

    /// Evaluates all cells in a given range and returns the results in a 2D vector.
    pub(crate) fn evaluate_range(
        &mut self,
        left: CellReferenceIndex,
        right: CellReferenceIndex,
    ) -> Vec<Vec<ArrayNode>> {
        let mut result = Vec::new();
        self.trace_rect(left.sheet, left.row, left.column, right.row, right.column);
        for r in left.row..=right.row {
            let mut row_result = Vec::new();
            for c in left.column..=right.column {
                let cell_reference = CellReferenceIndex {
                    sheet: left.sheet,
                    row: r,
                    column: c,
                };
                let value = match self.evaluate_cell(cell_reference) {
                    CalcResult::Number(n) => ArrayNode::Number(n),
                    CalcResult::Boolean(b) => ArrayNode::Boolean(b),
                    CalcResult::String(s) => ArrayNode::String(s),
                    CalcResult::Error { error, .. } => ArrayNode::Error(error),
                    CalcResult::EmptyCell | CalcResult::EmptyArg => ArrayNode::Empty,
                    CalcResult::Range { .. } | CalcResult::Array(_) | CalcResult::Lambda(_) => {
                        // This should never happen, but we need to handle it anyway
                        debug_assert!(false, "Unexpected array result in non-array formula");
                        ArrayNode::Error(Error::NIMPL)
                    }
                };
                row_result.push(value);
            }
            result.push(row_result);
        }
        result
    }

    #[inline(always)]
    fn fetch_cell(&self, cell_reference: CellReferenceIndex) -> Option<&Cell> {
        self.workbook.worksheets[cell_reference.sheet as usize]
            .sheet_data
            .get(&cell_reference.row)?
            .get(&cell_reference.column)
    }

    fn tracing(&self) -> bool {
        self.recalc_mode != RecalcMode::Full
    }

    fn trace_cell(&mut self, sheet: u32, row: i32, column: i32) {
        if let Some(top) = self.read_stack.last_mut() {
            top.record_cell((sheet, row, column));
        }
    }

    pub(crate) fn trace_rect(
        &mut self,
        sheet: u32,
        row1: i32,
        column1: i32,
        row2: i32,
        column2: i32,
    ) {
        if let Some(top) = self.read_stack.last_mut() {
            top.record_rect((sheet, row1, column1, row2, column2));
        }
    }

    pub(crate) fn trace_input(&mut self, input: Input) {
        if let Some(top) = self.read_stack.last_mut() {
            top.record_input(input);
        }
    }

    /// Used dimension of `sheet`. Records `SheetStructure` so a structural
    /// edit re-runs formulas that clipped a whole-column/row reference.
    pub(crate) fn sheet_dimension(
        &mut self,
        sheet: u32,
    ) -> Result<crate::worksheet::WorksheetDimension, String> {
        self.trace_input(Input::SheetStructure);
        Ok(self.workbook.worksheet(sheet)?.dimension())
    }

    /// Whether `row` is hidden. Records `RowHidden`.
    pub(crate) fn row_hidden(&mut self, sheet: u32, row: i32) -> Result<bool, String> {
        self.trace_input(Input::RowHidden(sheet, row));
        self.workbook.worksheet(sheet)?.is_row_hidden(row)
    }

    /// Table definition by name. Records `SheetStructure`.
    pub(crate) fn table_by_name(&mut self, name: &str) -> Option<&crate::types::Table> {
        self.trace_input(Input::SheetStructure);
        self.workbook.tables.get(name)
    }

    /// All tables. Records `SheetStructure`.
    pub(crate) fn tables(&mut self) -> &std::collections::HashMap<String, crate::types::Table> {
        self.trace_input(Input::SheetStructure);
        &self.workbook.tables
    }

    pub(crate) fn workbook_name(&self) -> &str {
        &self.workbook.name
    }

    pub(crate) fn sheet_count(&mut self) -> usize {
        self.trace_input(Input::SheetStructure);
        self.workbook.worksheets.len()
    }

    pub(crate) fn worksheet_name(&mut self, sheet: u32) -> Result<String, String> {
        Ok(self.workbook.worksheet(sheet)?.name.clone())
    }

    /// Formula index of a cell, if any. Records `FormulaText`.
    pub(crate) fn formula_index_at(&mut self, sheet: u32, row: i32, column: i32) -> Option<i32> {
        self.trace_input(Input::FormulaText((sheet, row, column)));
        self.workbook
            .worksheets
            .get(sheet as usize)?
            .cell(row, column)?
            .get_formula()
    }

    fn commit_reads(&mut self, dependent: (u32, i32, i32), reads: ReadSet) {
        if !self.tracing() {
            return;
        }
        self.graph.replace_reads(dependent, reads);
    }

    // Evaluates a cell and returns the value in the cell
    // FIXME: CalcResult cannot be Array or Range, should we have a different type?
    pub(crate) fn evaluate_cell(&mut self, cell_reference: CellReferenceIndex) -> CalcResult {
        self.trace_cell(
            cell_reference.sheet,
            cell_reference.row,
            cell_reference.column,
        );
        // A cell inside an array's footprint holds a value its anchor wrote, so
        // reading it is a read of the anchor. Nothing else records that: the
        // anchor's writes into its footprint are evaluation writes, not edits.
        // Taken from the array index rather than the cell, so a position whose
        // spill cell a structural edit dropped still names its anchor until the
        // next full pass refills it, and recorded before the scope gate below,
        // which can serve the stored value without ever reaching the anchor.
        // Without this edge a cycle closing through an array footprint is
        // invisible to the graph.
        if !self.graph.arrays.is_empty() && self.tracing() {
            let position = (
                cell_reference.sheet,
                cell_reference.row,
                cell_reference.column,
            );
            if let Some(anchor) = self.graph.arrays.anchor_of(position) {
                if anchor != position {
                    self.trace_cell(anchor.0, anchor.1, anchor.2);
                }
            }
        }
        // Incremental pass: a cell outside the affected set did not change, so
        // return its stored value instead of recomputing it (and its precedents).
        if let Some(scope) = &self.recompute_scope {
            let position = (
                cell_reference.sheet,
                cell_reference.row,
                cell_reference.column,
            );
            if !scope.contains(&position) {
                return match self.fetch_cell(cell_reference) {
                    Some(cell) => self.get_cell_value(cell, cell_reference),
                    None => CalcResult::EmptyCell,
                };
            }
        }

        let original_cell = match self.fetch_cell(cell_reference) {
            Some(c) => c.clone(),
            None => return CalcResult::EmptyCell,
        };

        if let Cell::SpillCell { a, .. } = original_cell {
            // If it is part of an array or dynamic formula we need to evaluate the anchor cell
            // strictly speaking we don't need to evaluate the anchor cell of a dynamic array formula
            // but it is most likely a good guess anyway
            let anchor_cell_reference = CellReferenceIndex {
                sheet: cell_reference.sheet,
                column: a.1,
                row: a.0,
            };
            // evaluate the anchor and discard the result
            let _ = self.evaluate_cell(anchor_cell_reference);
            // refetch the cell after evaluating the spill reference
            let cell = match self.fetch_cell(cell_reference) {
                Some(c) => c,
                None => return CalcResult::EmptyCell,
            };
            // and return its value
            return self.get_cell_value(cell, cell_reference);
        };

        match original_cell.get_formula() {
            Some(f) => {
                let key = (
                    cell_reference.sheet,
                    cell_reference.row,
                    cell_reference.column,
                );
                if let Some(state) = self.cells.get(&key) {
                    match state {
                        CellState::Evaluating => {
                            // The incremental scheduler watches this: a cycle
                            // the dependency graph did not already contain was
                            // ordered wrong, and only a full pass reproduces
                            // Full's `#CIRC!` placement.
                            self.saw_circular_reference = true;
                            return CalcResult::new_error(
                                Error::CIRC,
                                cell_reference,
                                "Circular reference detected".to_string(),
                            );
                        }
                        CellState::Evaluated => {
                            return self.get_cell_value(&original_cell, cell_reference);
                        }
                    }
                }
                // Clear the pre-existing spill area of a dynamic formula before re-evaluating.
                // This must happen after the CellState check so that a recursive call from a
                // spill cell does not wipe out spill cells that were just written.
                if let Cell::ArrayFormula {
                    r,
                    kind: ArrayKind::Dynamic,
                    ..
                } = &original_cell
                {
                    let (width, height) = *r;
                    let ws = match self.workbook.worksheet_mut(cell_reference.sheet) {
                        Ok(ws) => ws,
                        Err(_) => {
                            return CalcResult::new_error(
                                Error::ERROR,
                                cell_reference,
                                "Invalid sheet".to_string(),
                            )
                        }
                    };
                    for r in cell_reference.row..cell_reference.row + height {
                        for c in cell_reference.column..cell_reference.column + width {
                            if r == cell_reference.row && c == cell_reference.column {
                                continue;
                            }
                            // Only clear cells that are spill cells belonging to this anchor.
                            // Non-SpillCell content must remain
                            // so they can block the spill on re-evaluation.
                            let is_own_spill = ws
                                .sheet_data
                                .get(&r)
                                .and_then(|row_data| row_data.get(&c))
                                .map(|cell| {
                                    matches!(cell, Cell::SpillCell { a, .. }
                                        if *a == (cell_reference.row, cell_reference.column))
                                })
                                .unwrap_or(false);
                            if is_own_spill {
                                let _ = ws.cell_clear_contents(r, c);
                            }
                        }
                    }
                }
                // mark cell as being evaluated
                self.cells.insert(key, CellState::Evaluating);
                if self.tracing() {
                    self.read_stack.push(ReadSet::default());
                }
                let (node, _static_result) =
                    &self.parsed_formulas[cell_reference.sheet as usize][f as usize];
                let result = self.evaluate_node_in_context(&node.clone(), cell_reference);

                // At this point a range needs to be transformed into an array
                let result = if let CalcResult::Range { left, right } = result {
                    if left.sheet == right.sheet
                        && left.row == right.row
                        && left.column == right.column
                    {
                        // it is a single cell range, we can just return the value of the cell
                        self.evaluate_cell(left)
                    } else if !matches!(original_cell, Cell::ArrayFormula { .. }) {
                        // A scalar formula produced a multi-cell range at runtime,
                        // e.g. a name defined as a range after the formula was
                        // parsed, so no `@` was inserted. Apply implicit
                        // intersection here, while the range's true coordinates
                        // are known; materializing it into an array loses them.
                        match implicit_intersection(&cell_reference, &Range { left, right }) {
                            Some(target) => self.evaluate_cell(target),
                            None => CalcResult::new_error(
                                Error::VALUE,
                                cell_reference,
                                "Implicit intersection found no value".to_string(),
                            ),
                        }
                    } else {
                        let array_height = right.row - left.row + 1;
                        let array_width = right.column - left.column + 1;
                        let last_row = cell_reference.row + array_height - 1;
                        let last_col = cell_reference.column + array_width - 1;
                        if last_row > LAST_ROW || last_col > LAST_COLUMN {
                            CalcResult::new_error(
                                Error::SPILL,
                                cell_reference,
                                "Spill would exceed worksheet bounds".to_string(),
                            )
                        } else {
                            let array = self.evaluate_range(left, right);
                            CalcResult::Array(array)
                        }
                    }
                } else if matches!(result, CalcResult::Lambda(_)) {
                    CalcResult::new_error(
                        Error::CALC,
                        cell_reference,
                        "A LAMBDA was returned but not called".to_string(),
                    )
                } else {
                    result
                };

                // Formula-result boundary: a blank result coerces to 0, matching
                // what `set_cells_with_result` stores below. Every reader —
                // same-pass dependent, out-of-cone incremental read, or stored
                // value — observes the same number, so Full is order-independent.
                let result = match result {
                    CalcResult::EmptyCell | CalcResult::EmptyArg => CalcResult::Number(0.0),
                    r => r,
                };

                if let Err(e) = self.set_cells_with_result(cell_reference, &original_cell, &result)
                {
                    self.cells.insert(key, CellState::Evaluated);
                    if self.tracing() {
                        if let Some(reads) = self.read_stack.pop() {
                            self.commit_reads(key, reads);
                        }
                    }
                    // TODO: I _think_ this can never happen. Maybe we should  refactor things in a way that this is apparent
                    return CalcResult::new_error(Error::ERROR, cell_reference, e);
                };

                // mark cell as evaluated
                self.cells.insert(key, CellState::Evaluated);
                if self.tracing() {
                    if let Some(reads) = self.read_stack.pop() {
                        self.commit_reads(key, reads);
                    }
                }

                // return the result of the evaluation.
                match result {
                    CalcResult::Array(a) => {
                        // The cell ended up holding an array. Dependents must
                        // observe the same scalar `set_cells_with_result` wrote:
                        // the top-left element. That is the anchor value for a
                        // CSE/dynamic formula and the coerced value for a computed
                        // array in a scalar context. An empty element reads as the
                        // stored 0, never as a blank.
                        if a.is_empty() || a[0].is_empty() {
                            CalcResult::new_error(
                                Error::CALC,
                                cell_reference,
                                "Formula produced a zero-size array".to_string(),
                            )
                        } else {
                            match a[0][0] {
                                ArrayNode::Number(n) => CalcResult::Number(n),
                                ArrayNode::Boolean(b) => CalcResult::Boolean(b),
                                ArrayNode::String(ref s) => CalcResult::String(s.clone()),
                                ArrayNode::Error(ref error) => {
                                    let message = error.to_localized_error_string(self.language);
                                    CalcResult::new_error(error.clone(), cell_reference, message)
                                }
                                ArrayNode::Empty => CalcResult::Number(0.0),
                            }
                        }
                    }
                    _ => result,
                }
            }
            None => self.get_cell_value(&original_cell, cell_reference),
        }
    }

    pub(crate) fn get_sheet_index_by_name(&self, name: &str) -> Option<u32> {
        let worksheets = &self.workbook.worksheets;
        for (index, worksheet) in worksheets.iter().enumerate() {
            if worksheet.get_name().to_uppercase() == name.to_uppercase() {
                return Some(index as u32);
            }
        }
        None
    }

    /// Returns a model from an internal binary representation of a workbook
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # use ironcalc_base::cell::CellValue;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// model.set_user_input(0, 1, 1, "Stella!".to_string());
    /// let model2 = Model::from_bytes(&model.to_bytes(), "en")?;
    /// assert_eq!(
    ///     model2.get_cell_value_by_index(0, 1, 1),
    ///     Ok(CellValue::String("Stella!".to_string()))
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See also:
    /// * [Model::to_bytes]
    pub fn from_bytes(s: &[u8], language_id: &'a str) -> Result<Model<'a>, String> {
        let workbook: Workbook =
            bitcode::decode(s).map_err(|e| format!("Error parsing workbook: {e}"))?;
        Model::from_workbook(workbook, language_id)
    }

    /// Returns a model from a Workbook object
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # use ironcalc_base::cell::CellValue;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// model.set_user_input(0, 1, 1, "Stella!".to_string());
    /// let model2 = Model::from_workbook(model.workbook, "en")?;
    /// assert_eq!(
    ///     model2.get_cell_value_by_index(0, 1, 1),
    ///     Ok(CellValue::String("Stella!".to_string()))
    /// );
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_workbook(workbook: Workbook, language_id: &str) -> Result<Model<'_>, String> {
        let parsed_formulas = Vec::new();
        let worksheets = &workbook.worksheets;

        let worksheet_names = worksheets.iter().map(|s| s.get_name()).collect();

        let defined_names = workbook.get_defined_names_with_scope();
        // add all tables
        // let mut tables = Vec::new();
        // for worksheet in worksheets {
        //     let mut tables_in_sheet = HashMap::new();
        //     for table in &worksheet.tables {
        //         tables_in_sheet.insert(table.name.clone(), table.clone());
        //     }
        //     tables.push(tables_in_sheet);
        // }

        let cells = HashMap::new();
        let locale =
            get_locale(&workbook.settings.locale).map_err(|_| "Invalid locale".to_string())?;
        let tz = Tz::parse(&workbook.settings.tz)?;

        let language = match get_language(language_id) {
            Ok(lang) => lang,
            Err(_) => return Err("Invalid language".to_string()),
        };
        let parser = Parser::new(
            worksheet_names,
            defined_names,
            workbook.tables.clone(),
            locale,
            language,
        );
        let mut shared_strings = HashMap::new();
        for (index, s) in workbook.shared_strings.iter().enumerate() {
            shared_strings.insert(s.to_string(), index);
        }

        let mut model = Model {
            workbook,
            parsed_formulas,
            shared_strings,
            parsed_defined_names: HashMap::new(),
            parser,
            cells,
            language,
            locale,
            tz,
            view_id: 0,
            variable_stack: HashMap::new(),
            last_variable_id: 0,
            lambdas: HashMap::new(),
            last_lambda_id: 0,
            spill_cells: Vec::new(),
            support: HashMap::new(),
            cf_cache: HashMap::new(),
            links: HashMap::new(),
            graph: DependencyGraph::default(),
            recalc_mode: RecalcMode::from_env(),
            recompute_scope: None,
            formula_cell_count: 0,
            formula_count_stale: false,
            wrote_array_cells: false,
            saw_circular_reference: false,
            cse_rects: None,
            cse_member_guard: cse_guard::CseMemberGuard::default(),
            read_stack: Vec::new(),
            changed_cells: ChangedCells::All,
            write_seeds: HashSet::new(),
        };

        model.parse_formulas();
        model.parse_defined_names();
        model.evaluate_conditional_formatting();

        Ok(model)
    }

    /// Parses a reference like "Sheet1!B4" into {0, 2, 4}
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # use ironcalc_base::expressions::types::CellReferenceIndex;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// model.set_user_input(0, 1, 1, "Stella!".to_string());
    /// let reference = model.parse_reference("Sheet1!D40");
    /// assert_eq!(reference, Some(CellReferenceIndex {sheet: 0, row: 40, column: 4}));
    /// # Ok(())
    /// # }
    /// ```
    pub fn parse_reference(&self, s: &str) -> Option<CellReferenceIndex> {
        let bytes = s.as_bytes();
        let mut sheet_name = "".to_string();
        let mut column = "".to_string();
        let mut row = "".to_string();
        let mut state = "sheet"; // "sheet", "col", "row"
        for &byte in bytes {
            match state {
                "sheet" => {
                    if byte == b'!' {
                        state = "col"
                    } else {
                        sheet_name.push(byte as char);
                    }
                }
                "col" => {
                    if byte.is_ascii_alphabetic() {
                        column.push(byte as char);
                    } else {
                        state = "row";
                        row.push(byte as char);
                    }
                }
                _ => {
                    row.push(byte as char);
                }
            }
        }
        let sheet = self.get_sheet_index_by_name(&sheet_name)?;
        let row = match row.parse::<i32>() {
            Ok(r) => r,
            Err(_) => return None,
        };
        if !(1..=constants::LAST_ROW).contains(&row) {
            return None;
        }

        let column = match utils::column_to_number(&column) {
            Ok(column) => {
                if is_valid_column_number(column) {
                    column
                } else {
                    return None;
                }
            }
            Err(_) => return None,
        };

        Some(CellReferenceIndex { sheet, row, column })
    }

    /// Moves the formula `value` from `source` (in `area`) to `target`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # use ironcalc_base::expressions::types::{Area, CellReferenceIndex};
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// let source = CellReferenceIndex { sheet: 0, row: 3, column: 1};
    /// let target = CellReferenceIndex { sheet: 0, row: 50, column: 1};
    /// let area = Area { sheet: 0, row: 1, column: 1, width: 5, height: 4};
    /// let result = model.move_cell_value_to_area("=B1", &source, &target, &area)?;
    /// assert_eq!(&result, "=B48");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See also:
    /// * [Model::extend_to()]
    /// * [Model::extend_copied_value()]
    pub fn move_cell_value_to_area(
        &mut self,
        value: &str,
        source: &CellReferenceIndex,
        target: &CellReferenceIndex,
        area: &Area,
    ) -> Result<String, String> {
        let source_sheet_name = self
            .workbook
            .worksheet(source.sheet)
            .map_err(|e| format!("Could not find source worksheet: {e}"))?
            .get_name();
        if source.sheet != area.sheet {
            return Err("Source and area are in different sheets".to_string());
        }
        if source.row < area.row || source.row >= area.row + area.height {
            return Err("Source is outside the area".to_string());
        }
        if source.column < area.column || source.column >= area.column + area.width {
            return Err("Source is outside the area".to_string());
        }
        let target_sheet_name = self
            .workbook
            .worksheet(target.sheet)
            .map_err(|e| format!("Could not find target worksheet: {e}"))?
            .get_name();
        if let Some(formula) = self.formula_without_prefix(value) {
            let cell_reference = CellReferenceRC {
                sheet: source_sheet_name.to_owned(),
                row: source.row,
                column: source.column,
            };
            let formula_str = move_formula(
                &self.parser.parse(formula, &cell_reference),
                &MoveContext {
                    source_sheet_name: &source_sheet_name,
                    row: source.row,
                    column: source.column,
                    area,
                    target_sheet_name: &target_sheet_name,
                    row_delta: target.row - source.row,
                    column_delta: target.column - source.column,
                },
                self.locale,
                self.language,
            );
            Ok(format!("={formula_str}"))
        } else {
            Ok(value.to_string())
        }
    }

    /// 'Extends' the value from cell (`sheet`, `row`, `column`) to (`target_row`, `target_column`) in the same sheet
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// let (sheet, row, column) = (0, 1, 1);
    /// model.set_user_input(sheet, row, column, "=B1*D4".to_string());
    /// let (target_row, target_column) = (30, 1);
    /// let result = model.extend_to(sheet, row, column, target_row, target_column)?;
    /// assert_eq!(&result, "=B30*D33");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See also:
    /// * [Model::extend_copied_value()]
    /// * [Model::move_cell_value_to_area()]
    pub fn extend_to(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
        target_row: i32,
        target_column: i32,
    ) -> Result<String, String> {
        let cell = self.workbook.worksheet(sheet)?.cell(row, column);
        let result = match cell {
            Some(cell) => match cell.get_formula() {
                None => cell.get_localized_text(
                    &self.workbook.shared_strings,
                    self.locale,
                    self.language,
                ),
                Some(i) => {
                    let (formula, _static_result) =
                        &self.parsed_formulas[sheet as usize][i as usize];
                    let cell_ref = CellReferenceRC {
                        sheet: self.workbook.worksheets[sheet as usize].get_name(),
                        row: target_row,
                        column: target_column,
                    };
                    format!(
                        "={}",
                        to_localized_string(formula, &cell_ref, self.locale, self.language)
                    )
                }
            },
            None => "".to_string(),
        };
        Ok(result)
    }

    /// 'Extends' the formula `value` from `source` to `target`
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # use ironcalc_base::expressions::types::CellReferenceIndex;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// let source = CellReferenceIndex {sheet: 0, row: 1, column: 1};
    /// let target = CellReferenceIndex {sheet: 0, row: 30, column: 1};
    /// let result = model.extend_copied_value("=B1*D4", &source, &target)?;
    /// assert_eq!(&result, "=B30*D33");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See also:
    /// * [Model::extend_to()]
    /// * [Model::move_cell_value_to_area()]
    pub fn extend_copied_value(
        &mut self,
        value: &str,
        source: &CellReferenceIndex,
        target: &CellReferenceIndex,
    ) -> Result<String, String> {
        let source_sheet_name = match self.workbook.worksheets.get(source.sheet as usize) {
            Some(ws) => ws.get_name(),
            None => {
                return Err("Invalid worksheet index".to_owned());
            }
        };
        let target_sheet_name = match self.workbook.worksheets.get(target.sheet as usize) {
            Some(ws) => ws.get_name(),
            None => {
                return Err("Invalid worksheet index".to_owned());
            }
        };

        if let Some(formula_str) = self.formula_without_prefix(value) {
            let cell_reference = CellReferenceRC {
                sheet: source_sheet_name.to_string(),
                row: source.row,
                column: source.column,
            };
            let formula = &self.parser.parse(formula_str, &cell_reference);
            let cell_reference = CellReferenceRC {
                sheet: target_sheet_name,
                row: target.row,
                column: target.column,
            };
            return Ok(format!(
                "={}",
                to_localized_string(formula, &cell_reference, self.locale, self.language)
            ));
        }
        Ok(value.to_string())
    }

    /// Returns the formula in (`sheet`, `row`, `column`) if any
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// let (sheet, row, column) = (0, 1, 1);
    /// model.set_user_input(sheet, row, column, "=SIN(B1*C3)+1".to_string());
    /// model.evaluate();
    /// let result = model.get_cell_formula(sheet, row, column)?;
    /// assert_eq!(result, Some("=SIN(B1*C3)+1".to_string()));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See also:
    /// * [Model::get_localized_cell_content()]
    pub fn get_cell_formula(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Option<String>, String> {
        let worksheet = self.workbook.worksheet(sheet)?;
        match worksheet.cell(row, column) {
            Some(cell) => match cell.get_formula() {
                Some(formula_index) => {
                    let (formula, _static_result) = &self
                        .parsed_formulas
                        .get(sheet as usize)
                        .ok_or("missing sheet")?
                        .get(formula_index as usize)
                        .ok_or("missing formula")?;
                    let cell_ref = CellReferenceRC {
                        sheet: worksheet.get_name(),
                        row,
                        column,
                    };
                    Ok(Some(format!(
                        "={}",
                        to_localized_string(formula, &cell_ref, self.locale, self.language)
                    )))
                }
                None => Ok(None),
            },
            None => Ok(None),
        }
    }

    /// Returns the text for the formula in (`sheet`, `row`, `column`) in English if any
    ///
    /// See also:
    /// * [Model::get_localized_cell_content()]
    pub(crate) fn get_english_cell_formula(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Option<String>, String> {
        let worksheet = self.workbook.worksheet(sheet)?;
        match worksheet.cell(row, column) {
            Some(cell) => match cell.get_formula() {
                Some(formula_index) => {
                    let (formula, _static_result) = &self
                        .parsed_formulas
                        .get(sheet as usize)
                        .ok_or("missing sheet")?
                        .get(formula_index as usize)
                        .ok_or("missing formula")?;
                    let cell_ref = CellReferenceRC {
                        sheet: worksheet.get_name(),
                        row,
                        column,
                    };
                    let language_en = get_default_language();
                    Ok(Some(format!(
                        "={}",
                        to_localized_string(formula, &cell_ref, self.locale, language_en)
                    )))
                }
                None => Ok(None),
            },
            None => Ok(None),
        }
    }

    /// Updates the value of a cell with some text
    /// It does not change the style unless needs to add "quoting"
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// let (sheet, row, column) = (0, 1, 1);
    /// model.set_user_input(sheet, row, column, "Hello!".to_string())?;
    /// assert_eq!(model.get_localized_cell_content(sheet, row, column)?, "Hello!".to_string());
    ///
    /// model.update_cell_with_text(sheet, row, column, "Goodbye!")?;
    /// assert_eq!(model.get_localized_cell_content(sheet, row, column)?, "Goodbye!".to_string());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See also:
    /// * [Model::set_user_input()]
    /// * [Model::update_cell_with_number()]
    /// * [Model::update_cell_with_bool()]
    /// * [Model::update_cell_with_formula()]
    pub fn update_cell_with_text(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        value: &str,
    ) -> Result<(), String> {
        if !is_valid_row(row) || !is_valid_column_number(column) {
            return Err("Incorrect row or column".to_string());
        }
        let style_index = self.get_cell_style_index(sheet, row, column)?;
        let new_style_index;
        if common::value_needs_quoting(value, self.language) {
            new_style_index = self
                .workbook
                .styles
                .get_style_with_quote_prefix(style_index)?;
        } else if self.workbook.styles.style_is_quote_prefix(style_index) {
            new_style_index = self
                .workbook
                .styles
                .get_style_without_quote_prefix(style_index)?;
        } else {
            new_style_index = style_index;
        }

        self.set_cell_with_string(sheet, row, column, value, new_style_index)
    }

    /// Updates the value of a cell with a boolean value
    /// It does not change the style
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// let (sheet, row, column) = (0, 1, 1);
    /// model.set_user_input(sheet, row, column, "TRUE".to_string())?;
    /// assert_eq!(model.get_localized_cell_content(sheet, row, column)?, "TRUE".to_string());
    ///
    /// model.update_cell_with_bool(sheet, row, column, false)?;
    /// assert_eq!(model.get_localized_cell_content(sheet, row, column)?, "FALSE".to_string());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See also:
    /// * [Model::set_user_input()]
    /// * [Model::update_cell_with_number()]
    /// * [Model::update_cell_with_text()]
    /// * [Model::update_cell_with_formula()]
    pub fn update_cell_with_bool(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        value: bool,
    ) -> Result<(), String> {
        if !is_valid_row(row) || !is_valid_column_number(column) {
            return Err("Incorrect row or column".to_string());
        }
        let style_index = self.get_cell_style_index(sheet, row, column)?;
        let new_style_index = if self.workbook.styles.style_is_quote_prefix(style_index) {
            self.workbook
                .styles
                .get_style_without_quote_prefix(style_index)?
        } else {
            style_index
        };
        self.set_cell_with_boolean(sheet, row, column, value, new_style_index)
    }

    /// Updates the value of a cell with a number
    /// It does not change the style
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// let (sheet, row, column) = (0, 1, 1);
    /// model.set_user_input(sheet, row, column, "42".to_string())?;
    /// assert_eq!(model.get_localized_cell_content(sheet, row, column)?, "42".to_string());
    ///
    /// model.update_cell_with_number(sheet, row, column, 23.0)?;
    /// assert_eq!(model.get_localized_cell_content(sheet, row, column)?, "23".to_string());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See also:
    /// * [Model::set_user_input()]
    /// * [Model::update_cell_with_text()]
    /// * [Model::update_cell_with_bool()]
    /// * [Model::update_cell_with_formula()]
    pub fn update_cell_with_number(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        value: f64,
    ) -> Result<(), String> {
        if !is_valid_row(row) || !is_valid_column_number(column) {
            return Err("Incorrect row or column".to_string());
        }
        let style_index = self.get_cell_style_index(sheet, row, column)?;
        let new_style_index = if self.workbook.styles.style_is_quote_prefix(style_index) {
            self.workbook
                .styles
                .get_style_without_quote_prefix(style_index)?
        } else {
            style_index
        };
        self.set_cell_with_number(sheet, row, column, value, new_style_index)
    }

    /// Updates the formula of given cell
    /// It does not change the style unless needs to add "quoting"
    /// Expects the formula to start with "="
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// let (sheet, row, column) = (0, 1, 1);
    /// model.set_user_input(sheet, row, column, "=A2*2".to_string())?;
    /// model.evaluate();
    /// assert_eq!(model.get_localized_cell_content(sheet, row, column)?, "=A2*2".to_string());
    ///
    /// model.update_cell_with_formula(sheet, row, column, "=A3*2".to_string())?;
    /// model.evaluate();
    /// assert_eq!(model.get_localized_cell_content(sheet, row, column)?, "=A3*2".to_string());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See also:
    /// * [Model::set_user_input()]
    /// * [Model::update_cell_with_number()]
    /// * [Model::update_cell_with_bool()]
    /// * [Model::update_cell_with_text()]
    pub fn update_cell_with_formula(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        formula: String,
    ) -> Result<(), String> {
        self.write_formula_bytes(sheet, row, column, formula)
    }

    /// Rewrite a formula after a structural displacement. Does not rebuild the graph.
    ///
    /// Logged as `is_formula: false` so the journal consumer dirties the cell
    /// instead of force-fulling. Displacement is not a user formula edit:
    /// `record_structural_edit` already shifts the existing edges.
    pub(crate) fn write_displaced_formula(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        formula: String,
    ) -> Result<(), String> {
        let was_formula = self
            .workbook
            .worksheet(sheet)?
            .cell(row, column)
            .and_then(Cell::get_formula)
            .is_some();
        let result = {
            // The raw write would be journaled as a formula edit. Substitute
            // our own entry below instead of suppressing it.
            let mut paused = self.pause_journal_for_sheet(sheet);
            paused.write_formula_bytes(sheet, row, column, formula)
        };
        if result.is_ok() {
            self.workbook.worksheets[sheet as usize]
                .write_log
                .push(Write::Cell {
                    row,
                    column,
                    was_formula,
                    is_formula: false,
                });
        }
        result
    }

    fn write_formula_bytes(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        formula: String,
    ) -> Result<(), String> {
        let mut style_index = self.get_cell_style_index(sheet, row, column)?;
        if self.workbook.styles.style_is_quote_prefix(style_index) {
            style_index = self
                .workbook
                .styles
                .get_style_without_quote_prefix(style_index)?;
        }

        if let Some(new_formula) = self.formula_without_prefix(&formula) {
            self.set_cell_with_formula(sheet, row, column, new_formula, style_index)?;
            Ok(())
        } else {
            Err(format!("\"{formula}\" is not a valid formula"))
        }
    }

    // If we are writing in (sheet, row, column). If it is:
    // - A single cell => do nothing
    // - Part of an array formula => we bail
    // - Anchor of an array formula => we delete the formula and we clear the spill
    // - Part of a dynamic array formula => we delete the formula and we clear the spill
    // - Anchor of a dynamic array formula
    //     => we clear the spill and we set an unevaluated dynamic formula.
    fn prepare_cell_for_user_input(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<(), String> {
        match self.get_cell_structure(sheet, row, column)? {
            CellStructure::SingleCell => {
                // A structural edit can drop a CSE member cell while the
                // anchor still owns and refills its rectangle; the position is
                // still part of the array, and writing there would be silently
                // undone by the next evaluation.
                if !self.cse_member_guard.is_suspended()
                    && self.covered_by_cse_rect(sheet, row, column)
                {
                    return Err(
                        "Cannot write in a cell that is part of an array formula".to_string()
                    );
                }
            }
            CellStructure::ArrayFormula { range } => {
                // We cannot write in a cell that is part of an array formula
                let (width, height) = range;
                if width > 1 || height > 1 {
                    return Err(
                        "Cannot write in a cell that is part of an array formula".to_string()
                    );
                }
            }
            CellStructure::DynamicFormula { range } => {
                // clear the spill of the dynamic formula
                let (width, height) = range;
                self.workbook
                    .worksheet_mut(sheet)?
                    .clear_array_footprint(row, column, width, height, false);
            }
            CellStructure::SpillArray { .. } => {
                return Err("Cannot write in a cell that is part of an array formula".to_string());
            }
            CellStructure::SpillDynamic { anchor, range } => {
                // It is part of a dynamic array formula, but it is not the anchor.
                // We can write in it but we need to clear the spill and reset the anchor
                // to an unevaluated dynamic formula so it will re-spill on next evaluate().
                let (anchor_row, anchor_column) = anchor;
                let (width, height) = range;
                let ws = self.workbook.worksheet_mut(sheet)?;
                // Extract formula index and style from the anchor before mutating
                let (formula_index, anchor_style) = {
                    let anchor_cell = ws
                        .cell(anchor_row, anchor_column)
                        .ok_or_else(|| "Dynamic formula anchor not found".to_string())?;
                    let fi = anchor_cell
                        .get_formula()
                        .ok_or_else(|| "Dynamic formula anchor has no formula".to_string())?;
                    let s = anchor_cell.get_style();
                    (fi, s)
                };
                ws.set_cell_with_dynamic_formula(
                    anchor_row,
                    anchor_column,
                    formula_index,
                    anchor_style,
                    1,
                    1,
                )?;
                ws.clear_array_footprint(anchor_row, anchor_column, width, height, true);
            }
        };
        Ok(())
    }

    /// Sets a cell parametrized by (`sheet`, `row`, `column`) with `value`.
    ///
    /// This mimics a user entering a value on a cell.
    ///
    /// If you enter a currency `$100` it will set as a number and update the style
    ///  Note that for currencies/percentage there is only one possible style
    ///  The value is always a string, so we need to try to cast it into numbers/booleans/errors
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # use ironcalc_base::cell::CellValue;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// model.set_user_input(0, 1, 1, "100$".to_string());
    /// model.set_user_input(0, 2, 1, "125$".to_string());
    /// model.set_user_input(0, 3, 1, "-10$".to_string());
    /// model.set_user_input(0, 1, 2, "=SUM(A:A)".to_string());
    /// model.evaluate();
    /// assert_eq!(model.get_cell_value_by_index(0, 1, 2), Ok(CellValue::Number(215.0)));
    /// assert_eq!(model.get_formatted_cell_value(0, 1, 2), Ok("215$".to_string()));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// See also:
    /// * [Model::update_cell_with_formula()]
    /// * [Model::update_cell_with_number()]
    /// * [Model::update_cell_with_bool()]
    /// * [Model::update_cell_with_text()]
    pub fn set_user_input(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        value: String,
    ) -> Result<(), String> {
        // Reject invalid coordinates before touching the graph: a dirty seed at a
        // missing sheet panics on the next Incremental evaluate, and an invalid
        // row/column would appear in the changed-cell delta without a write.
        if !is_valid_row(row) || !is_valid_column_number(column) {
            return Err("Row or column is outside valid range.".to_string());
        }
        let _ = self.workbook.worksheet(sheet)?;
        // first we make sure we can write in the cell and clear the spills.
        self.prepare_cell_for_user_input(sheet, row, column)?;
        if value.is_empty() {
            // If the value is empty we just clear the cell.
            // Deleting the contents of a cell also removes its link.
            let ws = self.workbook.worksheet_mut(sheet)?;
            ws.cell_clear_contents(row, column)?;
            if ws.links.remove(&(row, column)).is_some() {
                ws.write_log.push(Write::Link {
                    at: (sheet, row, column),
                });
            }
            return Ok(());
        }

        // If value starts with "'" then we force the style to be quote_prefix
        let style_index = self.get_cell_style_index(sheet, row, column)?;
        if let Some(new_value) = value.strip_prefix('\'') {
            let new_style = self
                .workbook
                .styles
                .get_style_with_quote_prefix(style_index)?;
            self.set_cell_with_string(sheet, row, column, new_value, new_style)?;
        } else {
            let mut new_style_index = style_index;
            if self.workbook.styles.style_is_quote_prefix(style_index) {
                new_style_index = self
                    .workbook
                    .styles
                    .get_style_without_quote_prefix(style_index)?;
            }
            if let Some(formula) = self.formula_without_prefix(&value) {
                let formula_index =
                    self.set_cell_with_formula(sheet, row, column, formula, new_style_index)?;
                // Update the style if needed
                let cell = CellReferenceIndex { sheet, row, column };
                let (parsed_formula, _static_result) =
                    &self.parsed_formulas[sheet as usize][formula_index as usize];
                if let Some(units) = self.compute_node_units(parsed_formula, &cell) {
                    let new_style_index = self
                        .workbook
                        .styles
                        .get_style_with_format(new_style_index, &units.get_num_fmt())?;
                    let style = self.workbook.styles.get_style(new_style_index)?;
                    self.set_cell_style(sheet, row, column, &style)?;
                }
            } else {
                // The list of currencies is '$', '€' and the local currency
                let mut currencies = vec!["$", "€"];
                let currency = &self.locale.currency.symbol;
                if !currencies.iter().any(|e| e == currency) {
                    currencies.push(currency);
                }

                //  We try to parse as number
                if let Ok((v, number_format)) =
                    parse_formatted_number(&value, &currencies, self.locale)
                {
                    if let Some(num_fmt) = number_format {
                        // Should not apply the format in the following cases:
                        // - we assign a date to already date-formatted cell
                        let should_apply_format = !(is_likely_date_number_format(
                            &self.workbook.styles.get_style(new_style_index)?.num_fmt,
                        ) && is_likely_date_number_format(&num_fmt));
                        if should_apply_format {
                            new_style_index = self
                                .workbook
                                .styles
                                .get_style_with_format(new_style_index, &num_fmt)?;
                        }
                    }
                    let worksheet = self.workbook.worksheet_mut(sheet)?;
                    worksheet.set_cell_with_number(row, column, v, new_style_index)?;
                    return Ok(());
                }
                // We try to parse as boolean
                if let Ok(v) = value.to_lowercase().parse::<bool>() {
                    let worksheet = self.workbook.worksheet_mut(sheet)?;
                    worksheet.set_cell_with_boolean(row, column, v, new_style_index)?;
                    return Ok(());
                }
                // Check is it is error value
                let upper = value.to_uppercase();
                let worksheet = self.workbook.worksheet_mut(sheet)?;
                match get_error_by_name(&upper, self.language) {
                    Some(error) => {
                        worksheet.set_cell_with_error(row, column, error, new_style_index)?;
                    }
                    None => {
                        self.set_cell_with_string(sheet, row, column, &value, new_style_index)?;
                        // If the input looks like an URL or an email address a link is
                        // attached to the cell, the same way other inputs change the
                        // number format. Note that a quote prefix prevents this.
                        self.auto_link_cell(sheet, row, column, &value)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Sets an array formula in an area (CSE formula)
    pub fn set_user_array_formula(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        width: i32,
        height: i32,
        value: &str,
    ) -> Result<(), String> {
        self.prepare_cell_for_user_input(sheet, row, column)?;
        // A new CSE rectangle may be about to exist; rebuild the memo on the
        // next membership query rather than waiting for the journal drain.
        self.cse_rects = None;
        // If value starts with "'" then we force the style to be quote_prefix
        let style_index = self.get_cell_style_index(sheet, row, column)?;
        if value.strip_prefix('\'').is_none() {
            let mut new_style_index = style_index;
            if self.workbook.styles.style_is_quote_prefix(style_index) {
                new_style_index = self
                    .workbook
                    .styles
                    .get_style_without_quote_prefix(style_index)?;
            }
            if let Some(formula) = value.strip_prefix('=') {
                // It is a formula, we mark it as an array formulas and fill the "spills" with placeholders
                let formula_index = self.set_cell_with_array_formula(
                    sheet,
                    row,
                    column,
                    formula,
                    new_style_index,
                    width,
                    height,
                )?;

                // Update the style if needed
                let cell = CellReferenceIndex { sheet, row, column };
                let (parsed_formula, _static_result) =
                    &self.parsed_formulas[sheet as usize][formula_index as usize];

                if let Some(units) = self.compute_node_units(parsed_formula, &cell) {
                    let new_style_index = self
                        .workbook
                        .styles
                        .get_style_with_format(new_style_index, &units.get_num_fmt())?;
                    let style = self.workbook.styles.get_style(new_style_index)?;
                    self.set_cell_style(sheet, row, column, &style)?;
                }
                // Update the "spill" area with placeholders
                for r in row..row + height {
                    for c in column..column + width {
                        if r == row && c == column {
                            continue;
                        }
                        let mut new_style_index_spill = self.get_cell_style_index(sheet, r, c)?;
                        if self
                            .workbook
                            .styles
                            .style_is_quote_prefix(new_style_index_spill)
                        {
                            new_style_index_spill = self
                                .workbook
                                .styles
                                .get_style_without_quote_prefix(new_style_index_spill)?;
                        }

                        self.set_cell_with_string(sheet, r, c, "", new_style_index_spill)?;
                    }
                }
                return Ok(());
            }
        }
        // just use set user input on every cell
        for r in row..row + height {
            for c in column..column + width {
                self.set_user_input(sheet, r, c, value.to_string())?;
            }
        }

        Ok(())
    }

    pub(crate) fn get_cell_structure(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<CellStructure, String> {
        let worksheet = self.workbook.worksheet(sheet)?;
        worksheet.get_cell_structure(row, column)
    }

    fn set_cell_with_formula(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        formula: &str,
        style: i32,
    ) -> Result<i32, String> {
        let worksheet = self.workbook.worksheet_mut(sheet)?;
        let cell_reference = CellReferenceRC {
            sheet: worksheet.get_name(),
            row,
            column,
        };
        let shared_formulas = &mut worksheet.shared_formulas;
        let mut parsed_formula = self.parser.parse(formula, &cell_reference);
        // If the formula fails to parse try adding a parenthesis
        // SUM(A1:A3  => SUM(A1:A3)
        if let Node::ParseErrorKind { .. } = parsed_formula {
            let new_parsed_formula = self.parser.parse(&format!("{formula})"), &cell_reference);
            match new_parsed_formula {
                Node::ParseErrorKind { .. } => {}
                _ => parsed_formula = new_parsed_formula,
            }
        }
        let static_result = run_static_analysis_on_node(&parsed_formula);
        let is_dynamic = !matches!(static_result, StaticResult::Scalar);

        let s = to_rc_format(&parsed_formula);
        let mut formula_index: i32 = -1;
        if let Some(index) = shared_formulas.iter().position(|x| x == &s) {
            formula_index = index as i32;
        }
        if formula_index == -1 {
            shared_formulas.push(s);
            self.parsed_formulas[sheet as usize].push((parsed_formula, static_result));
            formula_index = (shared_formulas.len() as i32) - 1;
        }
        if is_dynamic {
            worksheet.set_cell_with_dynamic_formula(row, column, formula_index, style, 1, 1)?;
        } else {
            worksheet.set_cell_with_formula(row, column, formula_index, style)?;
        }
        Ok(formula_index)
    }

    // FIXME
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_cell_with_array_formula(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        formula: &str,
        style: i32,
        width: i32,
        height: i32,
    ) -> Result<i32, String> {
        let worksheet = self.workbook.worksheet_mut(sheet)?;
        let cell_reference = CellReferenceRC {
            sheet: worksheet.get_name(),
            row,
            column,
        };
        let shared_formulas = &mut worksheet.shared_formulas;
        let mut parsed_formula = self.parser.parse(formula, &cell_reference);
        // If the formula fails to parse try adding a parenthesis
        // SUM(A1:A3  => SUM(A1:A3)
        if let Node::ParseErrorKind { .. } = parsed_formula {
            let new_parsed_formula = self.parser.parse(&format!("{formula})"), &cell_reference);
            match new_parsed_formula {
                Node::ParseErrorKind { .. } => {}
                _ => parsed_formula = new_parsed_formula,
            }
        }
        let static_result = run_static_analysis_on_node(&parsed_formula);

        let s = to_rc_format(&parsed_formula);
        let mut formula_index: i32 = -1;
        if let Some(index) = shared_formulas.iter().position(|x| x == &s) {
            formula_index = index as i32;
        }
        if formula_index == -1 {
            shared_formulas.push(s);
            self.parsed_formulas[sheet as usize].push((parsed_formula, static_result));
            formula_index = (shared_formulas.len() as i32) - 1;
        }
        worksheet.set_cell_with_array_formula(row, column, formula_index, style, width, height)?;
        Ok(formula_index)
    }

    pub(crate) fn set_cell_with_string(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        value: &str,
        style: i32,
    ) -> Result<(), String> {
        match self.shared_strings.get(value) {
            Some(string_index) => {
                self.workbook.worksheet_mut(sheet)?.set_cell_with_string(
                    row,
                    column,
                    *string_index as i32,
                    style,
                )?;
            }
            None => {
                let string_index = self.workbook.shared_strings.len();
                self.workbook.shared_strings.push(value.to_string());
                self.shared_strings.insert(value.to_string(), string_index);
                self.workbook.worksheet_mut(sheet)?.set_cell_with_string(
                    row,
                    column,
                    string_index as i32,
                    style,
                )?;
            }
        }
        Ok(())
    }

    fn set_cell_with_boolean(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        value: bool,
        style: i32,
    ) -> Result<(), String> {
        self.workbook
            .worksheet_mut(sheet)?
            .set_cell_with_boolean(row, column, value, style)
    }

    fn set_cell_with_number(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        value: f64,
        style: i32,
    ) -> Result<(), String> {
        self.workbook
            .worksheet_mut(sheet)?
            .set_cell_with_number(row, column, value, style)
    }

    // Helper function that returns a defined name given the name and scope
    fn get_parsed_defined_name(
        &self,
        name: &str,
        scope: Option<u32>,
    ) -> Result<Option<ParsedDefinedName>, String> {
        let name_upper = name.to_uppercase();

        for (key, df) in &self.parsed_defined_names {
            if key.1.to_uppercase() == name_upper && key.0 == scope {
                return Ok(Some(df.clone()));
            }
        }
        Ok(None)
    }

    // Returns the formula for a defined name
    pub(crate) fn get_defined_name_formula(
        &self,
        name: &str,
        scope: Option<u32>,
    ) -> Result<String, String> {
        let name_upper = name.to_uppercase();
        let defined_names = &self.workbook.defined_names;
        let sheet_id = match scope {
            Some(index) => Some(self.workbook.worksheet(index)?.sheet_id),
            None => None,
        };
        for df in defined_names {
            if df.name.to_uppercase() == name_upper && df.sheet_id == sheet_id {
                return Ok(df.formula.clone());
            }
        }
        Err("Defined name not found".to_string())
    }

    /// Returns the list of defined names as `(name, scope, formula)`.
    ///
    /// Formulas are stored internally in English; they are translated into the
    /// active language/locale for display.
    pub fn get_defined_name_list(&self) -> Vec<(String, Option<u32>, String)> {
        let context = self.defined_name_context();
        self.workbook
            .get_defined_names_with_scope()
            .into_iter()
            .map(|(name, scope, formula)| {
                let formula = self.internal_formula_to_display(&formula, &context);
                (name, scope, formula)
            })
            .collect()
    }

    /// Gets the Excel Value (Bool, Number, String) of a cell
    ///
    /// See also:
    /// * [Model::get_cell_value_by_index()]
    pub fn get_cell_value_by_ref(&self, cell_ref: &str) -> Result<CellValue, String> {
        let cell_reference = match self.parse_reference(cell_ref) {
            Some(c) => c,
            None => return Err(format!("Error parsing reference: '{cell_ref}'")),
        };
        let sheet_index = cell_reference.sheet;
        let column = cell_reference.column;
        let row = cell_reference.row;

        self.get_cell_value_by_index(sheet_index, row, column)
    }

    /// Returns the cell value for (`sheet`, `row`, `column`)
    ///
    /// See also:
    /// * [Model::get_formatted_cell_value()]
    pub fn get_cell_value_by_index(
        &self,
        sheet_index: u32,
        row: i32,
        column: i32,
    ) -> Result<CellValue, String> {
        let cell = self
            .workbook
            .worksheet(sheet_index)?
            .cell(row, column)
            .cloned()
            .unwrap_or_default();
        let cell_value = cell.value(&self.workbook.shared_strings, self.language);
        Ok(cell_value)
    }

    /// Returns the formatted cell value for (`sheet`, `row`, `column`)
    ///
    /// See also:
    /// * [Model::get_cell_value_by_index()]
    /// * [Model::get_cell_value_by_ref]
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// let (sheet, row, column) = (0, 1, 1);
    /// model.set_user_input(sheet, row, column, "=1/3".to_string());
    /// model.evaluate();
    /// let result = model.get_formatted_cell_value(sheet, row, column)?;
    /// assert_eq!(result, "0.333333333".to_string());
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_formatted_cell_value(
        &self,
        sheet_index: u32,
        row: i32,
        column: i32,
    ) -> Result<String, String> {
        match self.workbook.worksheet(sheet_index)?.cell(row, column) {
            Some(cell) => {
                let format = self.get_style_for_cell(sheet_index, row, column)?.num_fmt;
                let formatted_value =
                    cell.formatted_value(&self.workbook.shared_strings, self.language, |value| {
                        format_number(value, &format, self.locale).text
                    });
                Ok(formatted_value)
            }
            None => Ok("".to_string()),
        }
    }

    /// Return the typeof a cell
    pub fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Result<CellType, String> {
        Ok(match self.workbook.worksheet(sheet)?.cell(row, column) {
            Some(c) => c.get_type(),
            None => CellType::Number,
        })
    }

    /// Returns a string with the cell content in the given language and locale.
    /// If there is a formula returns the formula
    /// If the cell is empty returns the empty string
    /// Returns an error if there is no worksheet
    /// If the cell has quote prefix style it adds a ' at the beginning of the value
    /// If the cell is date formatted it tries to format it as date
    pub fn get_localized_cell_content(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<String, String> {
        let worksheet = self.workbook.worksheet(sheet)?;
        let cell = match worksheet.cell(row, column) {
            Some(c) => c,
            None => return Ok("".to_string()),
        };
        match cell.get_formula() {
            Some(formula_index) => {
                let formula = &self.parsed_formulas[sheet as usize][formula_index as usize].0;
                let cell_ref = CellReferenceRC {
                    sheet: worksheet.get_name(),
                    row,
                    column,
                };
                Ok(format!(
                    "={}",
                    to_localized_string(formula, &cell_ref, self.locale, self.language)
                ))
            }
            None => {
                let style_index = cell.get_style();
                let style = self.workbook.styles.get_style(style_index)?;
                if style.quote_prefix {
                    Ok(format!(
                        "'{}",
                        cell.get_localized_text(
                            &self.workbook.shared_strings,
                            self.locale,
                            self.language,
                        )
                    ))
                } else {
                    // If it is a date formatted cell we try to format it as date, if it fails we return the raw value
                    if is_likely_date_number_format(&style.num_fmt) {
                        let value = cell.value(&self.workbook.shared_strings, self.language);
                        if let CellValue::Number(n) = value {
                            let formatted = format_number(n, &style.num_fmt, self.locale);
                            if formatted.error.is_none() {
                                return Ok(formatted.text);
                            }
                        }
                    }
                    Ok(cell.get_localized_text(
                        &self.workbook.shared_strings,
                        self.locale,
                        self.language,
                    ))
                }
            }
        }
    }

    /// The stored cell at `position`, if any. `None` for a blank cell and for
    /// an out-of-range sheet alike: neither has content to read.
    pub(crate) fn cell_at(&self, (sheet, row, column): Position) -> Option<&Cell> {
        self.workbook
            .worksheet(sheet)
            .ok()
            .and_then(|ws| ws.cell(row, column))
    }

    /// Every stored cell in the workbook, in `(sheet, row, column)` order.
    ///
    /// `sheet_data` is a hash map, so the sort is what makes the order exist at
    /// all. This is the order the full pass evaluates in and the order every
    /// index built from a whole-workbook walk is built in, so it has one
    /// definition. A caller that needs `&mut self` afterwards collects first.
    /// The full pass walks this twice over every cell in the workbook, so the
    /// sort carries the cell reference it already has rather than the column
    /// number to look the cell up by again: the same order, one hash lookup per
    /// cell instead of two.
    pub(crate) fn cells_in_order(&self) -> impl Iterator<Item = (Position, &Cell)> + '_ {
        self.workbook
            .worksheets
            .iter()
            .enumerate()
            .flat_map(|(sheet_index, worksheet)| {
                let mut sorted_rows: Vec<i32> = worksheet.sheet_data.keys().copied().collect();
                sorted_rows.sort_unstable();
                sorted_rows.into_iter().flat_map(move |row| {
                    let row_data = &worksheet.sheet_data[&row];
                    let mut sorted_columns: Vec<(i32, &Cell)> = row_data
                        .iter()
                        .map(|(&column, cell)| (column, cell))
                        .collect();
                    sorted_columns.sort_unstable_by_key(|&(column, _)| column);
                    sorted_columns
                        .into_iter()
                        .map(move |(column, cell)| ((sheet_index as u32, row, column), cell))
                })
            })
    }

    /// Returns a list of all cells
    pub fn get_all_cells(&self) -> Vec<CellIndex> {
        self.cells_in_order()
            .map(|((index, row, column), _)| CellIndex { index, row, column })
            .collect()
    }

    /// Collects the cells the full pass evaluates in phase 1, in natural
    /// (sheet, row, column) order, and stores them in `self.spill_cells`.
    ///
    /// This is one of the two selections on [`is_phase_one_cell`]; the other is
    /// [`Model::in_full_pass_order`], which reproduces this order over a cone.
    fn collect_spill_cells(&mut self) {
        self.spill_cells = self
            .cells_in_order()
            .filter(|(_, cell)| is_phase_one_cell(cell))
            .map(|((sheet, row, column), _)| CellReferenceIndex { sheet, row, column })
            .collect();
    }

    /// Returns all cells in the current spill area of a dynamic-formula anchor,
    /// including the anchor itself.
    fn get_spill_area(&self, cell_ref: CellReferenceIndex) -> Vec<CellReferenceIndex> {
        let ws = match self.workbook.worksheet(cell_ref.sheet) {
            Ok(ws) => ws,
            Err(_) => return Vec::new(),
        };
        let (width, height) = match ws.cell(cell_ref.row, cell_ref.column) {
            Some(Cell::ArrayFormula {
                r,
                kind: ArrayKind::Dynamic,
                ..
            }) => *r,
            _ => return Vec::new(),
        };
        (cell_ref.row..cell_ref.row + height)
            .flat_map(|r| {
                (cell_ref.column..cell_ref.column + width).map(move |c| CellReferenceIndex {
                    sheet: cell_ref.sheet,
                    row: r,
                    column: c,
                })
            })
            .collect()
    }

    /// Returns true if any position in `positions` falls within a dependency of `cell`.
    fn position_in_support(
        &self,
        cell: CellReferenceIndex,
        positions: &[CellReferenceIndex],
    ) -> bool {
        let deps = match self.support.get(&cell) {
            Some(d) => d,
            None => return false,
        };
        for dep in deps {
            match *dep {
                CellOrRange::Cell((sheet, row, col)) => {
                    if positions
                        .iter()
                        .any(|p| p.sheet == sheet && p.row == row && p.column == col)
                    {
                        return true;
                    }
                }
                CellOrRange::Range((sheet, r1, c1, r2, c2)) => {
                    if positions.iter().any(|p| {
                        p.sheet == sheet
                            && p.row >= r1
                            && p.row <= r2
                            && p.column >= c1
                            && p.column <= c2
                    }) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Recomputes the workbook using the configured [`RecalcMode`] (`Full` by
    /// default).
    pub fn evaluate(&mut self) {
        self.drain_write_journal();
        let mode = self.recalc_mode;
        // Storing a formula result is not a user edit, so the journal stays
        // paused for the whole pass. The guard restores it however the pass
        // ends, panic included.
        let mut evaluating = self.pause_journal();
        match mode {
            RecalcMode::Full => evaluating.evaluate_full_untracked(),
            RecalcMode::Incremental => {
                evaluating.evaluate_selective();
            }
            #[cfg(feature = "recalc_verify")]
            RecalcMode::Verify => evaluating.verify_incremental_matches_full(),
        }
    }

    pub(crate) fn drain_write_journal(&mut self) {
        let mut writes = Vec::new();
        for (sheet_index, ws) in self.workbook.worksheets.iter_mut().enumerate() {
            let sheet = sheet_index as u32;
            for write in ws.write_log.drain() {
                writes.push((sheet, write));
            }
        }
        let mut first_was: std::collections::HashMap<Position, bool> =
            std::collections::HashMap::new();
        for (sheet, write) in writes {
            match write {
                Write::Cell {
                    row,
                    column,
                    was_formula,
                    is_formula,
                } => {
                    // The worksheet named a position inside itself; `sheet`
                    // comes from the enumeration above, which is the only
                    // thing that knows which log this was.
                    let p = (sheet, row, column);
                    // The first write of a batch carries the pre-batch
                    // formula-ness; the cell itself carries the final state.
                    first_was.entry(p).or_insert(was_formula);
                    if was_formula {
                        self.graph.remove_dependent(p);
                    }
                    self.graph.mark_dirty(p);
                    self.write_seeds.insert(p);
                    // FORMULATEXT/ISFORMULA read formula-ness, not the value.
                    // A number-to-number write does not change that.
                    if was_formula || is_formula {
                        let text_readers = self.graph.dependents_of_inputs(
                            |i| matches!(i, Input::FormulaText(q) if *q == p),
                        );
                        for r in text_readers {
                            self.graph.mark_dirty(r);
                        }
                    }
                }
                Write::Link { at } => {
                    self.graph.mark_dirty(at);
                    self.write_seeds.insert(at);
                }
                Write::Hidden { row, column } => {
                    let deps = if let Some(r) = row {
                        self.graph.dependents_of_inputs(
                            |i| matches!(i, Input::RowHidden(s, rr) if *s == sheet && *rr == r),
                        )
                    } else if let Some(c) = column {
                        self.graph.dependents_of_inputs(
                            |i| matches!(i, Input::ColHidden(s, cc) if *s == sheet && *cc == c),
                        )
                    } else {
                        std::collections::HashSet::new()
                    };
                    for p in deps {
                        self.graph.mark_dirty(p);
                    }
                }
            }
        }
        // Account journaled writes against the formula count and the array
        // index, so incremental passes need no whole-workbook walk. The cell
        // holds the batch's final state; `first_was` its pre-batch formula-ness.
        // A formula that stopped existing leaves a stale array-index entry,
        // which at worst forces a conservative Full fallback that rebuilds the
        // index exactly.
        for ((sheet, row, column), was_formula) in first_was {
            // Not `cell_at`: that borrows all of `self`, and the CSE-rect
            // memo is invalidated below while this borrow is still live.
            let cell = self
                .workbook
                .worksheet(sheet)
                .ok()
                .and_then(|ws| ws.cell(row, column));
            let is_formula_now = cell.map(|c| c.get_formula().is_some()).unwrap_or(false);
            let mut footprint = Vec::new();
            if let Some(cell) = cell {
                if matches!(
                    cell,
                    Cell::ArrayFormula {
                        kind: ArrayKind::Cse,
                        ..
                    }
                ) {
                    self.cse_rects = None;
                }
                array_index::array_footprint(cell, sheet, row, column, &mut |p, anchor| {
                    footprint.push((p, anchor))
                });
            }
            match (was_formula, is_formula_now) {
                (false, true) => self.formula_cell_count += 1,
                (true, false) => {
                    self.formula_cell_count = self.formula_cell_count.saturating_sub(1)
                }
                _ => {}
            }
            for (p, anchor) in footprint {
                self.graph.arrays.insert(p, anchor);
            }
        }
    }

    /// Chooses the recalculation strategy for the model's lifetime. See
    /// [`RecalcMode`]. Meant to be chained onto a constructor; forces the next
    /// evaluation to be full so the graph is built under the chosen strategy.
    #[must_use]
    pub fn with_recalc_mode(mut self, mode: RecalcMode) -> Self {
        self.recalc_mode = mode;
        self.graph.force_full();
        self
    }

    /// Non-cell invalidation: locale, timezone, or a full reparse.
    pub(crate) fn invalidate_graph(&mut self) {
        self.graph.force_full();
        self.cse_rects = None;
    }

    /// Whether `(row, column)` lies inside some CSE array's declared rectangle
    /// on `sheet`, excluding the anchor itself. Builds the rectangle list on
    /// demand; anchors are few, so the covering test is a short linear scan.
    /// A hit is validated against the live anchor cell: an anchor can be
    /// deleted without an invalidation in between (a paste that covers the
    /// whole array), in which case the memo rebuilds once and the test
    /// re-runs.
    fn covered_by_cse_rect(&mut self, sheet: u32, row: i32, column: i32) -> bool {
        for _attempt in 0..2 {
            if self.cse_rects.is_none() {
                let mut rects = Vec::new();
                for (sheet_index, worksheet) in self.workbook.worksheets.iter().enumerate() {
                    for (r, row_data) in &worksheet.sheet_data {
                        for (c, cell) in row_data {
                            if let Cell::ArrayFormula {
                                kind: ArrayKind::Cse,
                                r: (width, height),
                                ..
                            } = cell
                            {
                                rects.push((sheet_index as u32, *r, *c, *width, *height));
                            }
                        }
                    }
                }
                self.cse_rects = Some(rects);
            }
            let hit = self.cse_rects.as_ref().and_then(|rects| {
                rects.iter().copied().find(|&(s, r, c, w, h)| {
                    s == sheet
                        && row >= r
                        && row < r + h
                        && column >= c
                        && column < c + w
                        && !(row == r && column == c)
                })
            });
            let Some((s, r, c, w, h)) = hit else {
                return false;
            };
            let anchor_live = matches!(
                self.cell_at((s, r, c)),
                Some(Cell::ArrayFormula {
                    kind: ArrayKind::Cse,
                    r: (width, height),
                    ..
                }) if (*width, *height) == (w, h)
            );
            if anchor_live {
                return true;
            }
            self.cse_rects = None;
        }
        false
    }

    /// Recomputes every cell with the two-phase algorithm that handles dynamic
    /// arrays.
    ///
    /// Phase 1 evaluates spill-capable cells first, in dependency order, so their
    /// spill areas are populated before other cells read them. When a spill cell
    /// writes into a position an earlier spill cell depends on, the two are
    /// reordered and the phase restarts; an N*N bound prevents infinite loops on
    /// circular spill dependencies. Phase 2 evaluates the remaining cells.
    fn evaluate_full(&mut self) {
        if self.tracing() {
            self.graph.clear_edges();
        }
        self.collect_spill_cells();

        let n = self.spill_cells.len();
        // Each restart fixes at least one pair; O(N*N) restarts suffice.
        let max_restarts = n * n + 1;
        let mut retry = true;
        let mut restart_count = 0;

        while retry && restart_count < max_restarts {
            retry = false;
            self.cells.clear();
            self.support.clear();
            // dynamic links (HYPERLINK) are rebuilt on every evaluation
            self.links.clear();
            self.clear_variable_stack();
            self.clear_lambdas();

            // Phase 1: evaluate spill cells, correcting their order when needed.
            for i in 0..self.spill_cells.len() {
                let spill_cell = self.spill_cells[i];
                self.evaluate_cell(spill_cell);

                // Find every cell position written by this spill (anchor + spill cells).
                let spill_area = self.get_spill_area(spill_cell);

                // If any of those positions is a dependency of a spill cell that was
                // evaluated earlier (index j < i), the current cell must come first.
                for j in 0..i {
                    let prev = self.spill_cells[j];
                    if self.position_in_support(prev, &spill_area) {
                        let moved = self.spill_cells.remove(i);
                        self.spill_cells.insert(j, moved);
                        retry = true;
                        restart_count += 1;
                        break;
                    }
                }
                if retry {
                    break;
                }
            }
        }

        // Phase 2: evaluate everything else; spill cells are already Evaluated and skipped.
        // Fallback when max restarts is exceeded (circular spill dependency).
        let all_cells = self.get_all_cells();
        for cell in all_cells {
            self.evaluate_cell(CellReferenceIndex {
                sheet: cell.index,
                row: cell.row,
                column: cell.column,
            });
        }
        self.evaluate_conditional_formatting();
        // Only the incremental path reads the graph; Full mode skips building it.
        if self.recalc_mode != RecalcMode::Full {
            self.collect_array_cells();
            // This pass rebuilt every edge, so the never-served set is rebuilt
            // over the whole graph: a cycle anywhere in the workbook has to be
            // known here, because later incremental passes only look at the
            // cone their seeds reach, and a cycle they do not know about is one
            // they never seed.
            let nodes = self.graph.nodes();
            let cone = self.graph.cycle_cone(&nodes);
            self.graph.set_never_served(cone);
            self.refresh_blocked_array_readers();
            // Edges come from the read tracer (commit_reads during evaluate_cell).
            self.graph.after_pass();
        } else {
            // Ready + an ever-growing dirty set would make a later Incremental
            // switch (tests, with_recalc_mode) see stale seeds. Full has no
            // valid graph, so the next Incremental pass must rebuild.
            self.graph.force_full();
        }
    }

    /// Removes the content of every cell in the range but leaves the style.
    ///
    /// See also:
    /// * [Model::range_clear_all()]
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # use ironcalc_base::expressions::types::Area;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// let (sheet, row, column) = (0, 1, 1);
    /// model.set_user_input(sheet, row, column, "100$".to_string());
    /// let area = Area {
    ///     sheet,
    ///     row,
    ///     column,
    ///     width: 1,
    ///     height: 1,
    /// };
    /// model.range_clear_contents(&area)?;
    /// model.set_user_input(sheet, row, column, "10".to_string());
    /// let result = model.get_formatted_cell_value(sheet, row, column)?;
    /// assert_eq!(result, "10$".to_string());
    /// # Ok(())
    /// # }
    /// ```
    pub fn range_clear_contents(&mut self, range: &Area) -> Result<(), String> {
        if !self.can_clear_range(range)? {
            return Err("Cannot clear the range because it contains array formulas".to_string());
        }
        let sheet = range.sheet;
        let ws = self.workbook.worksheet_mut(sheet)?;
        for row in range.row..range.row + range.height {
            for column in range.column..range.column + range.width {
                let structure = ws.get_cell_structure(row, column)?;
                match structure {
                    CellStructure::DynamicFormula { range }
                    | CellStructure::ArrayFormula { range, .. } => {
                        let (width, height) = range;
                        ws.clear_array_footprint(row, column, width, height, false);
                    }
                    _ => {
                        let _ = ws.cell_clear_contents(row, column);
                    }
                }
            }
        }
        // Deleting the contents of a cell also removes its link. Each removal
        // is journaled: a stranded link can sit at a position with no cell, so
        // the cell clears above do not cover it and the delta would miss it.
        let removed_links: Vec<(i32, i32)> = ws
            .links
            .keys()
            .copied()
            .filter(|&(row, column)| {
                row >= range.row
                    && row < range.row + range.height
                    && column >= range.column
                    && column < range.column + range.width
            })
            .collect();
        for (row, column) in removed_links {
            ws.links.remove(&(row, column));
            ws.write_log.push(Write::Link {
                at: (range.sheet, row, column),
            });
        }
        Ok(())
    }

    // Returns true if for every array formula in the range, the whole spill is included in the range,
    // false otherwise.
    pub(crate) fn can_clear_range(&self, range: &Area) -> Result<bool, String> {
        let sheet = range.sheet;
        for row in range.row..range.row + range.height {
            for column in range.column..range.column + range.width {
                match self.get_cell_structure(sheet, row, column)? {
                    CellStructure::ArrayFormula { range: r } => {
                        let (width, height) = r;
                        if column + width > range.column + range.width
                            || row + height > range.row + range.height
                        {
                            return Ok(false);
                        }
                    }
                    CellStructure::SpillArray {
                        anchor: a,
                        range: r,
                    } => {
                        let (anchor_row, anchor_column) = a;
                        let (width, height) = r;
                        if anchor_column < range.column
                            || anchor_row < range.row
                            || anchor_column + width > range.column + range.width
                            || anchor_row + height > range.row + range.height
                        {
                            return Ok(false);
                        }
                    }
                    _ => {
                        // noop
                    }
                }
            }
        }
        Ok(true)
    }

    /// Deletes a range by removing it from worksheet data. All content and style is removed.
    /// It fails if it deletes part of an array formula.
    /// Deletes the whole spill if it is part of a dynamic array formula.
    ///
    /// See also:
    /// * [Model::range_clear_contents()]
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use ironcalc_base::Model;
    /// # use ironcalc_base::expressions::types::Area;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut model = Model::new_empty("model", "en", "UTC", "en")?;
    /// let (sheet, row, column) = (0, 1, 1);
    /// model.set_user_input(sheet, row, column, "100$".to_string());
    /// let area = Area {
    ///     sheet,
    ///     row,
    ///     column,
    ///     width: 1,
    ///     height: 1,
    /// };
    /// model.range_clear_all(&area)?;
    /// model.set_user_input(sheet, row, column, "10".to_string());
    /// let result = model.get_formatted_cell_value(sheet, row, column)?;
    /// assert_eq!(result, "10".to_string());
    /// # Ok(())
    /// # }
    pub fn range_clear_all(&mut self, area: &Area) -> Result<(), String> {
        if !self.can_clear_range(area)? {
            return Err("Cannot clear the range because it contains array formulas".to_string());
        }
        // The spill of a dynamic array reached from the range goes away whole,
        // but only the part of it inside the range was selected for clearing.
        // The cells outside keep their style, so the footprint is torn down
        // with the style-preserving `Worksheet::clear_array_footprint` instead
        // of a removal, which would drop it.
        let mut spills: Vec<(i32, i32, i32, i32)> = Vec::new();
        {
            let worksheet = self.workbook.worksheet(area.sheet)?;
            for row in area.row..area.row + area.height {
                for column in area.column..area.column + area.width {
                    if let Some(Cell::ArrayFormula {
                        r,
                        kind: ArrayKind::Dynamic,
                        ..
                    }) = worksheet.cell(row, column)
                    {
                        let (width, height) = *r;
                        spills.push((row, column, width, height));
                    }
                }
            }
        }
        let worksheet = self.workbook.worksheet_mut(area.sheet)?;
        // Cells in the range lose content and style alike. This runs before the
        // spill teardown so a spill cell inside the range is cleared of its own
        // style first and only inherits the row/column one.
        for row in area.row..area.row + area.height {
            for column in area.column..area.column + area.width {
                // Sanctioned: the user selected these cells, so they lose content and
                // style alike. The spill reaching outside the range is torn down below
                // with the style-preserving `clear_array_footprint`.
                #[allow(clippy::disallowed_methods)]
                let _ = worksheet.remove_cell(row, column);
            }
        }
        for (row, column, width, height) in spills {
            // The anchor is inside the range, so its removal above is undone
            // by the teardown re-materializing an `EmptyCell` there, exactly
            // as it is for the footprint's other in-range cells.
            worksheet.clear_array_footprint(row, column, width, height, false);
        }
        // Deleting the cells also removes their links. Each removal is
        // journaled: a stranded link can sit at a position with no cell, so
        // the cell removals above do not cover it and the delta would miss it.
        let removed_links: Vec<(i32, i32)> = worksheet
            .links
            .keys()
            .copied()
            .filter(|&(row, column)| {
                row >= area.row
                    && row < area.row + area.height
                    && column >= area.column
                    && column < area.column + area.width
            })
            .collect();
        for (row, column) in removed_links {
            worksheet.links.remove(&(row, column));
            worksheet.write_log.push(Write::Link {
                at: (area.sheet, row, column),
            });
        }
        Ok(())
    }

    /// Tears down dynamic-array spills that reach `boundary` on `axis`.
    /// Spills entirely above/left of the edit are left alone so an insert
    /// below a spill can stay incremental. Reset anchors are marked dirty;
    /// `evaluate_selective` then takes the array/spill full path.
    pub(crate) fn reset_dynamic_array_spills(
        &mut self,
        sheet: u32,
        axis: Axis,
        boundary: i32,
    ) -> Result<(), String> {
        // Collect anchor info first — can't mutate sheet_data while iterating over it.
        let anchors: Vec<(i32, i32, i32, i32, i32, i32)> = {
            let ws = self.workbook.worksheet(sheet)?;
            let mut result = Vec::new();
            for (row, row_data) in &ws.sheet_data {
                for (column, cell) in row_data {
                    if let Cell::ArrayFormula {
                        r,
                        f,
                        s,
                        kind: ArrayKind::Dynamic,
                        v,
                    } = cell
                    {
                        let (width, height) = *r;
                        let reaches = match axis {
                            Axis::Row => *row + height > boundary,
                            Axis::Column => *column + width > boundary,
                        };
                        // A blocked spill stores r=(1,1) and has no edge from the
                        // blocker, so `reaches` is false. The blocker moving away
                        // must still re-evaluate the anchor.
                        let blocked = matches!(
                            v,
                            FormulaValue::Error {
                                ei: Error::SPILL,
                                ..
                            }
                        );
                        if reaches || blocked {
                            result.push((*row, *column, *f, *s, width, height));
                        }
                    }
                }
            }
            result
        };

        for (row, column, f, s, width, height) in anchors {
            let ws = self.workbook.worksheet_mut(sheet)?;
            // Reset the anchor cell to DynamicFormula with r = (1, 1).
            // Goes through update_cell so the journal sees the formula rewrite.
            let _ = ws.update_cell(
                row,
                column,
                Cell::ArrayFormula {
                    f,
                    s,
                    r: (1, 1),
                    kind: ArrayKind::Dynamic,
                    v: FormulaValue::Unevaluated,
                },
            );
            // Delete all spill cells, keeping the just-rewritten anchor.
            ws.clear_array_footprint(row, column, width, height, true);
        }
        Ok(())
    }

    /// Returns the style index for cell (`sheet`, `row`, `column`)
    pub fn get_cell_style_index(&self, sheet: u32, row: i32, column: i32) -> Result<i32, String> {
        // First check the cell, then row, the column
        let cell = self.workbook.worksheet(sheet)?.cell(row, column);

        match cell {
            Some(cell) => Ok(cell.get_style()),
            None => {
                let rows = &self.workbook.worksheet(sheet)?.rows;
                for r in rows {
                    if r.r == row {
                        if r.custom_format {
                            return Ok(r.s);
                        }
                        break;
                    }
                }
                let cols = &self.workbook.worksheet(sheet)?.cols;
                for c in cols.iter() {
                    let min = c.min;
                    let max = c.max;
                    if column >= min && column <= max {
                        return Ok(c.style.unwrap_or(0));
                    }
                }
                Ok(0)
            }
        }
    }

    /// Returns the style for cell (`sheet`, `row`, `column`)
    /// If the cell does not have a style defined we check the row, otherwise the column and finally a default
    pub fn get_style_for_cell(&self, sheet: u32, row: i32, column: i32) -> Result<Style, String> {
        let style_index = self.get_cell_style_index(sheet, row, column)?;
        let style = self.workbook.styles.get_style(style_index)?;
        Ok(style)
    }

    /// Returns the style defined in a cell if any.
    pub fn get_cell_style_or_none(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Option<Style>, String> {
        let style = self
            .workbook
            .worksheet(sheet)?
            .cell(row, column)
            .map(|c| self.workbook.styles.get_style(c.get_style()))
            .transpose();
        style
    }

    /// Returns an internal binary representation of the workbook
    ///
    /// See also:
    /// * [Model::from_bytes]
    pub fn to_bytes(&self) -> Vec<u8> {
        bitcode::encode(&self.workbook)
    }

    /// Returns data about the worksheets
    pub fn get_worksheets_properties(&self) -> Vec<SheetProperties> {
        self.workbook
            .worksheets
            .iter()
            .map(|worksheet| SheetProperties {
                name: worksheet.get_name(),
                state: worksheet.state.to_string(),
                color: worksheet.color.clone(),
                sheet_id: worksheet.sheet_id,
            })
            .collect()
    }

    /// Returns markup representation of the given `sheet`.
    pub fn get_sheet_markup(&self, sheet: u32) -> Result<String, String> {
        let worksheet = self.workbook.worksheet(sheet)?;
        let dimension = worksheet.dimension();

        let mut rows = Vec::new();

        for row in 1..(dimension.max_row + 1) {
            let mut row_markup: Vec<String> = Vec::new();

            for column in 1..(dimension.max_column + 1) {
                let mut cell_markup = match self.get_cell_formula(sheet, row, column)? {
                    Some(formula) => formula,
                    None => self.get_formatted_cell_value(sheet, row, column)?,
                };
                let style = self.get_style_for_cell(sheet, row, column)?;
                if style.font.b {
                    cell_markup = format!("**{cell_markup}**")
                }
                row_markup.push(cell_markup);
            }

            rows.push(row_markup.join("|"));
        }

        Ok(rows.join("\n"))
    }

    /// Returns the number of frozen rows in `sheet`
    pub fn get_frozen_rows_count(&self, sheet: u32) -> Result<i32, String> {
        if let Some(worksheet) = self.workbook.worksheets.get(sheet as usize) {
            Ok(worksheet.frozen_rows)
        } else {
            Err("Invalid sheet".to_string())
        }
    }

    /// Return the number of frozen columns in `sheet`
    pub fn get_frozen_columns_count(&self, sheet: u32) -> Result<i32, String> {
        if let Some(worksheet) = self.workbook.worksheets.get(sheet as usize) {
            Ok(worksheet.frozen_columns)
        } else {
            Err("Invalid sheet".to_string())
        }
    }

    /// Sets the number of frozen rows to `frozen_rows` in the workbook.
    /// Fails if `frozen`_rows` is either too small (<0) or too large (>LAST_ROW)`
    pub fn set_frozen_rows(&mut self, sheet: u32, frozen_rows: i32) -> Result<(), String> {
        if let Some(worksheet) = self.workbook.worksheets.get_mut(sheet as usize) {
            if frozen_rows < 0 {
                return Err("Frozen rows cannot be negative".to_string());
            }
            if frozen_rows >= LAST_ROW {
                return Err("Too many rows".to_string());
            }
            worksheet.frozen_rows = frozen_rows;
            Ok(())
        } else {
            Err("Invalid sheet".to_string())
        }
    }

    /// Sets the number of frozen columns to `frozen_column` in the workbook.
    /// Fails if `frozen`_columns` is either too small (<0) or too large (>LAST_COLUMN)`
    pub fn set_frozen_columns(&mut self, sheet: u32, frozen_columns: i32) -> Result<(), String> {
        if let Some(worksheet) = self.workbook.worksheets.get_mut(sheet as usize) {
            if frozen_columns < 0 {
                return Err("Frozen columns cannot be negative".to_string());
            }
            if frozen_columns >= LAST_COLUMN {
                return Err("Too many columns".to_string());
            }
            worksheet.frozen_columns = frozen_columns;
            Ok(())
        } else {
            Err("Invalid sheet".to_string())
        }
    }

    /// Returns the width of a column
    #[inline]
    pub fn get_column_width(&self, sheet: u32, column: i32) -> Result<f64, String> {
        self.workbook.worksheet(sheet)?.get_column_width(column)
    }

    /// Sets the width of a column
    #[inline]
    pub fn set_column_width(&mut self, sheet: u32, column: i32, width: f64) -> Result<(), String> {
        self.workbook
            .worksheet_mut(sheet)?
            .set_column_width(column, width)
    }

    /// Sets whether a column is hidden
    #[inline]
    pub fn set_column_hidden(
        &mut self,
        sheet: u32,
        column: i32,
        hidden: bool,
    ) -> Result<(), String> {
        self.workbook
            .worksheet_mut(sheet)?
            .set_column_hidden(column, hidden)?;
        Ok(())
    }

    /// Sets whether a row is hidden
    #[inline]
    pub fn set_row_hidden(&mut self, sheet: u32, row: i32, hidden: bool) -> Result<(), String> {
        self.workbook
            .worksheet_mut(sheet)?
            .set_row_hidden(row, hidden)?;
        Ok(())
    }

    /// Returns whether a column is hidden
    #[inline]
    pub fn is_column_hidden(&self, sheet: u32, column: i32) -> Result<bool, String> {
        self.workbook.worksheet(sheet)?.is_column_hidden(column)
    }

    /// Returns whether a row is hidden
    #[inline]
    pub fn is_row_hidden(&self, sheet: u32, row: i32) -> Result<bool, String> {
        self.workbook.worksheet(sheet)?.is_row_hidden(row)
    }

    /// Returns the height of a row
    #[inline]
    pub fn get_row_height(&self, sheet: u32, row: i32) -> Result<f64, String> {
        self.workbook.worksheet(sheet)?.row_height(row)
    }

    /// Sets the height of a row
    #[inline]
    pub fn set_row_height(&mut self, sheet: u32, column: i32, height: f64) -> Result<(), String> {
        self.workbook
            .worksheet_mut(sheet)?
            .set_row_height(column, height)
    }

    /// Adds a new defined name.
    /// If scope is None it is a global defined name, otherwise it is local to the sheet with index scope.
    pub fn new_defined_name(
        &mut self,
        name: &str,
        scope: Option<u32>,
        formula: &str,
    ) -> Result<(), String> {
        let sheet_id = self.is_valid_defined_name(name, scope, formula)?;
        // Defined-name formulas are stored internally in English so they keep
        // working when the user switches language/locale.
        let context = self.defined_name_context();
        let internal_formula = self.user_formula_to_internal(formula, &context)?;
        self.workbook.defined_names.push(DefinedName {
            name: name.to_string(),
            formula: internal_formula,
            sheet_id,
        });
        self.reset_parsed_structures();

        Ok(())
    }

    /// The context used to parse/stringify defined-name formulas. Defined names
    /// have no natural anchor cell, so we use the first worksheet's A1.
    pub(crate) fn defined_name_context(&self) -> CellReferenceRC {
        CellReferenceRC {
            sheet: self
                .workbook
                .worksheets
                .first()
                .map(|ws| ws.get_name())
                .unwrap_or_else(|| "Sheet1".to_string()),
            row: 1,
            column: 1,
        }
    }

    /// Validates if a defined name can be created
    pub fn is_valid_defined_name(
        &mut self,
        name: &str,
        scope: Option<u32>,
        formula: &str,
    ) -> Result<Option<u32>, String> {
        if !is_valid_identifier(name) {
            return Err("Name: Invalid defined name".to_string());
        }
        let name_upper = name.to_uppercase();
        let defined_names = &self.workbook.defined_names;
        let sheet_id = match scope {
            Some(index) => match self.workbook.worksheet(index) {
                Ok(ws) => Some(ws.sheet_id),
                Err(_) => return Err("Scope: Invalid sheet index".to_string()),
            },
            None => None,
        };
        // if the defined name already exist return error
        for df in defined_names {
            if df.name.to_uppercase() == name_upper && df.sheet_id == sheet_id {
                return Err("Name: Defined name already exists".to_string());
            }
        }

        // Make sure the formula is valid — accept cell/range references OR a LAMBDA definition.
        let is_reference =
            common::ParsedReference::parse_reference_formula(None, formula, self.locale, |name| {
                self.get_sheet_index_by_name(name)
            })
            .is_ok();

        if !is_reference {
            // Try the full parser to see if it is a LAMBDA definition.
            // Defined-name formulas may carry a leading '='; strip it before parsing.
            use crate::expressions::types::CellReferenceRC;
            let formula_body = formula.strip_prefix('=').unwrap_or(formula);
            let dummy_ref = CellReferenceRC {
                sheet: self
                    .workbook
                    .worksheets
                    .first()
                    .map(|ws| ws.get_name())
                    .unwrap_or_else(|| "Sheet1".to_string()),
                row: 1,
                column: 1,
            };
            // Accept the formula whether it is written in the active language
            // or already in the internal English form (e.g. generated by
            // undo/redo or cut & paste).
            let mut node = self.parser.parse(formula_body, &dummy_ref);
            if let Node::ParseErrorKind { .. } = node {
                node = self.parse_internal_formula(formula_body, &dummy_ref);
            }
            if !matches!(node, Node::LambdaDefKind { .. }) {
                return Err("Formula: Invalid defined name formula".to_string());
            }
        }

        Ok(sheet_id)
    }

    /// Delete defined name of name and scope
    pub fn delete_defined_name(&mut self, name: &str, scope: Option<u32>) -> Result<(), String> {
        let name_upper = name.to_uppercase();
        let defined_names = &self.workbook.defined_names;
        let sheet_id = match scope {
            Some(index) => Some(self.workbook.worksheet(index)?.sheet_id),
            None => None,
        };
        let mut index = None;
        for (i, df) in defined_names.iter().enumerate() {
            if df.name.to_uppercase() == name_upper && df.sheet_id == sheet_id {
                index = Some(i);
            }
        }
        if let Some(i) = index {
            self.workbook.defined_names.remove(i);
            self.reset_parsed_structures();
            Ok(())
        } else {
            Err("Defined name not found".to_string())
        }
    }

    /// Update defined name
    pub fn update_defined_name(
        &mut self,
        name: &str,
        scope: Option<u32>,
        new_name: &str,
        new_scope: Option<u32>,
        new_formula: &str,
    ) -> Result<(), String> {
        if !is_valid_identifier(new_name) {
            return Err("Name: Invalid defined name".to_string());
        };
        let name_upper = name.to_uppercase();
        let new_name_upper = new_name.to_uppercase();

        if name_upper != new_name_upper || scope != new_scope {
            for key in self.parsed_defined_names.keys() {
                if key.1.to_uppercase() == new_name_upper && key.0 == new_scope {
                    return Err("Name: Defined name already exists".to_string());
                }
            }
        }
        let defined_names = &self.workbook.defined_names;
        let sheet_id = match scope {
            Some(index) => Some(
                self.workbook
                    .worksheet(index)
                    .map_err(|_| "Scope: Invalid sheet index")?
                    .sheet_id,
            ),
            None => None,
        };

        let new_sheet_id = match new_scope {
            Some(index) => Some(
                self.workbook
                    .worksheet(index)
                    .map_err(|_| "Scope: Invalid sheet index")?
                    .sheet_id,
            ),
            None => None,
        };

        let mut index = None;
        for (i, df) in defined_names.iter().enumerate() {
            if df.name.to_uppercase() == name_upper && df.sheet_id == sheet_id {
                index = Some(i);
            }
        }
        // Defined-name formulas are stored internally in English.
        let context = self.defined_name_context();
        let internal_formula = self.user_formula_to_internal(new_formula, &context)?;
        if let Some(i) = index {
            if let Some(df) = self.workbook.defined_names.get_mut(i) {
                if new_name != df.name {
                    // We need to rename the name in every formula:

                    // Parse all formulas with the old name
                    // All internal formulas are R1C1
                    self.parser.set_lexer_mode(LexerMode::R1C1);
                    let worksheets = &mut self.workbook.worksheets;
                    for worksheet in worksheets {
                        let cell_reference = CellReferenceRC {
                            sheet: worksheet.get_name(),
                            row: 1,
                            column: 1,
                        };
                        let mut formulas = Vec::new();
                        for formula in &worksheet.shared_formulas {
                            let mut t = self.parser.parse(formula, &cell_reference);
                            rename_defined_name_in_node(&mut t, name, scope, new_name);
                            formulas.push(to_rc_format(&t));
                        }
                        worksheet.shared_formulas = formulas;
                    }
                    // Se the mode back to A1
                    self.parser.set_lexer_mode(LexerMode::A1);
                }
                df.name = new_name.to_string();
                df.sheet_id = new_sheet_id;
                df.formula = internal_formula;
                self.reset_parsed_structures();
            }
            Ok(())
        } else {
            Err("Defined name not found".to_string())
        }
    }
    /// Returns the style object of a column, if any
    pub fn get_column_style(&self, sheet: u32, column: i32) -> Result<Option<Style>, String> {
        if let Some(worksheet) = self.workbook.worksheets.get(sheet as usize) {
            let cols = &worksheet.cols;
            for col in cols {
                if column >= col.min && column <= col.max {
                    if let Some(style_index) = col.style {
                        let style = self.workbook.styles.get_style(style_index)?;
                        return Ok(Some(style));
                    }
                    return Ok(None);
                }
            }
            Ok(None)
        } else {
            Err("Invalid sheet".to_string())
        }
    }

    /// Returns the style object of a row, if any
    pub fn get_row_style(&self, sheet: u32, row: i32) -> Result<Option<Style>, String> {
        if let Some(worksheet) = self.workbook.worksheets.get(sheet as usize) {
            let rows = &worksheet.rows;
            for r in rows {
                if row == r.r {
                    let style = self.workbook.styles.get_style(r.s)?;
                    return Ok(Some(style));
                }
            }
            Ok(None)
        } else {
            Err("Invalid sheet".to_string())
        }
    }

    /// Sets a column with style
    pub fn set_column_style(
        &mut self,
        sheet: u32,
        column: i32,
        style: &Style,
    ) -> Result<(), String> {
        let style_index = self.workbook.styles.get_style_index_or_create(style);
        self.workbook
            .worksheet_mut(sheet)?
            .set_column_style(column, style_index)
    }

    /// Sets a row with style
    pub fn set_row_style(&mut self, sheet: u32, row: i32, style: &Style) -> Result<(), String> {
        let style_index = self.workbook.styles.get_style_index_or_create(style);
        self.workbook
            .worksheet_mut(sheet)?
            .set_row_style(row, style_index)
    }

    /// Deletes the style of a column if the is any
    pub fn delete_column_style(&mut self, sheet: u32, column: i32) -> Result<(), String> {
        self.workbook
            .worksheet_mut(sheet)?
            .delete_column_style(column)
    }

    /// Deletes the style of a row if there is any
    pub fn delete_row_style(&mut self, sheet: u32, row: i32) -> Result<(), String> {
        self.workbook.worksheet_mut(sheet)?.delete_row_style(row)
    }

    /// Sets the locale of the model
    pub fn set_locale(&mut self, locale_id: &str) -> Result<(), String> {
        let locale = match get_locale(locale_id) {
            Ok(l) => l,
            Err(_) => return Err(format!("Invalid locale: {locale_id}")),
        };
        self.parser.set_locale(locale);
        self.locale = locale;
        self.workbook.settings.locale = locale_id.to_string();
        // A locale change re-renders every formatted value (TEXT, DOLLAR, dates).
        self.invalidate_graph();
        self.evaluate();
        Ok(())
    }

    /// Sets the timezone of the model
    pub fn set_timezone(&mut self, timezone: &str) -> Result<(), String> {
        let tz = match Tz::parse(timezone) {
            Ok(tz) => tz,
            Err(_) => return Err(format!("Invalid timezone: {}", timezone)),
        };
        self.tz = tz;
        self.workbook.settings.tz = timezone.to_string();
        // A timezone change moves NOW/TODAY and any time-formatted value.
        self.invalidate_graph();
        self.evaluate();
        Ok(())
    }

    /// Sets the language
    pub fn set_language(&mut self, language_id: &str) -> Result<(), String> {
        let language = match get_language(language_id) {
            Ok(l) => l,
            Err(_) => return Err(format!("Invalid language: {language_id}")),
        };
        self.parser.set_language(language);
        self.language = language;
        Ok(())
    }

    /// Gets the current language
    pub fn get_language(&self) -> String {
        self.language.code.clone()
    }

    /// Gets the timezone of the model
    pub fn get_timezone(&self) -> String {
        self.workbook.settings.tz.clone()
    }

    /// Gets the locale of the model
    pub fn get_locale(&self) -> String {
        self.workbook.settings.locale.clone()
    }

    /// Gets the formatting settings based on the locale
    pub fn get_fmt_settings(&self) -> FmtSettings {
        let day_example = 46006.0; // December 15, 2025
        let currency = self.locale.currency.iso.clone();
        let currency_symbol = &self.locale.currency.symbol;
        // "M/d/yy"
        let short_date = &self.locale.dates.date_formats.short;
        // "M/d/yyyy"
        let long_date = &self.locale.dates.date_formats.long;
        let short_date_example = format_number(day_example, short_date, self.locale).text;
        let long_date_example = format_number(day_example, long_date, self.locale).text;
        // Number format ("#,##0.###")
        // The CLDR formats are a bit different than Excel's
        // let number_fmt = self.locale.numbers.decimal_formats.standard.clone();
        // "#,##0.00 ¤" Currency format might have weird spaces
        let currency_format_template = &self.locale.numbers.currency_formats.standard;
        let currency_format = currency_format_template
            .replace("¤", &format!("\"{}\"", currency_symbol))
            .replace(" ", " ");

        let number_fmt = "#,##0.00".to_string();
        let number_example = format_number(1234.567, &number_fmt, self.locale).text;
        FmtSettings {
            currency,
            currency_format,
            short_date: short_date.clone(),
            long_date: long_date.clone(),
            short_date_example,
            long_date_example,
            number_fmt,
            number_example,
        }
    }

    /// Cycles the references touched by the cursor through the four
    /// absolute/relative states, Excel F4 style: A1 -> $A$1 -> A$1 -> $A1 -> A1.
    /// Returns the new text together with the new cursor start and end.
    ///
    /// Given cycle_reference("=A1", 3, 3) returns ("=$A$1", 5, 5)
    pub fn cycle_reference(
        &self,
        value: &str,
        start: usize,
        end: usize,
    ) -> Result<(String, i32, i32), String> {
        crate::expressions::lexer::util::cycle_reference(
            value,
            start,
            end,
            self.locale,
            self.language,
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::CellReferenceIndex as CellReference;
    use crate::{test::util::new_empty_model, types::Cell};

    #[test]
    fn test_cell_reference_to_string() {
        let model = new_empty_model();
        let reference = CellReference {
            sheet: 0,
            row: 32,
            column: 16,
        };
        assert_eq!(
            model.cell_reference_to_string(&reference),
            Ok("Sheet1!P32".to_string())
        )
    }

    #[test]
    fn test_cell_reference_to_string_invalid_worksheet() {
        let model = new_empty_model();
        let reference = CellReference {
            sheet: 10,
            row: 1,
            column: 1,
        };
        assert_eq!(
            model.cell_reference_to_string(&reference),
            Err("Invalid sheet index".to_string())
        )
    }

    #[test]
    fn test_cell_reference_to_string_invalid_column() {
        let model = new_empty_model();
        let reference = CellReference {
            sheet: 0,
            row: 1,
            column: 20_000,
        };
        assert_eq!(
            model.cell_reference_to_string(&reference),
            Err("Invalid column".to_string())
        )
    }

    #[test]
    fn test_cell_reference_to_string_invalid_row() {
        let model = new_empty_model();
        let reference = CellReference {
            sheet: 0,
            row: 2_000_000,
            column: 1,
        };
        assert_eq!(
            model.cell_reference_to_string(&reference),
            Err("Invalid row".to_string())
        )
    }

    #[test]
    fn test_get_cell() {
        let mut model = new_empty_model();
        model._set("A1", "35");
        model._set("A2", "");
        let worksheet = model.workbook.worksheet(0).expect("Invalid sheet");

        assert_eq!(
            worksheet.cell(1, 1),
            Some(&Cell::NumberCell { v: 35.0, s: 0 })
        );

        // Clears the content of A2 but not the style
        assert_eq!(worksheet.cell(2, 1), Some(&Cell::EmptyCell { s: 0 }));
        assert_eq!(worksheet.cell(3, 1), None)
    }

    #[test]
    fn test_get_cell_invalid_sheet() {
        let model = new_empty_model();
        assert_eq!(
            model.workbook.worksheet(5),
            Err("Invalid sheet index".to_string()),
        )
    }

    #[test]
    fn test_update_cell_with_sign_prefixed_formulas() {
        let mut model = new_empty_model();

        let update_result = model.update_cell_with_formula(0, 1, 1, "-A2*2".to_string());
        model.evaluate();
        assert_eq!(update_result, Ok(()));
        assert_eq!(model._get_formula("A1"), *"=-A2*2");
    }
}
