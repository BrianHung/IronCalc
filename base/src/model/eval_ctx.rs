//! The receiver formula evaluation runs on.
//!
//! `EvalCtx` is a newtype around `&mut Model` whose inner reference is private
//! to this module. `functions/` — and the evaluation support in `cast.rs` and
//! `arithmetic.rs` — take `EvalCtx` rather than `Model`, so the only workbook
//! state they can reach is what is re-exposed below, and everything re-exposed
//! below either routes through the tracer (`trace_cell` via `evaluate_cell`,
//! `trace_rect`, `trace_input`) or is not a read of cell state at all. There
//! are no exceptions: every accessor here that reads cell state records it.
//!
//! This is what makes invariant I1 — every read a formula evaluation performs
//! is recorded — constructional rather than conventional. `self.workbook` does
//! not resolve in `functions/`: the field belongs to `Model`, and `functions/`
//! never holds one.

use std::collections::HashMap;

use crate::{
    calc_result::CalcResult,
    expressions::{
        parser::{ArrayNode, Node},
        types::CellReferenceIndex,
    },
    locale::Locale,
    model::Model,
    recalc::Input,
    types::Table,
    worksheet::WorksheetDimension,
};

/// A `&mut Model` restricted to the operations formula evaluation may perform.
///
/// The wrapped reference is deliberately private: a method on `EvalCtx` is the
/// only way out, and no method hands back the `Workbook`.
pub(crate) struct EvalCtx<'a, 'm>(&'a mut Model<'m>);

impl<'m> Model<'m> {
    /// Borrow this model as the evaluation receiver.
    pub(crate) fn eval_ctx(&mut self) -> EvalCtx<'_, 'm> {
        EvalCtx(self)
    }
}

impl<'a, 'm> EvalCtx<'a, 'm> {
    // -- the tracer itself ------------------------------------------------

    pub(crate) fn trace_input(&mut self, input: Input) {
        self.0.trace_input(input);
    }

    pub(crate) fn trace_rect(
        &mut self,
        sheet: u32,
        row1: i32,
        column1: i32,
        row2: i32,
        column2: i32,
    ) {
        self.0.trace_rect(sheet, row1, column1, row2, column2);
    }

    // -- reads of cell state, each recording ------------------------------

    pub(crate) fn evaluate_cell(&mut self, cell_reference: CellReferenceIndex) -> CalcResult {
        self.0.evaluate_cell(cell_reference)
    }

    pub(crate) fn evaluate_range(
        &mut self,
        left: CellReferenceIndex,
        right: CellReferenceIndex,
    ) -> Vec<Vec<ArrayNode>> {
        self.0.evaluate_range(left, right)
    }

    /// Folds a rectangular range through `reducer`, reusing cached prefixes in
    /// incremental mode. Reads inside record their edges like any other.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fold_range(
        &mut self,
        sheet: u32,
        row1: i32,
        column1: i32,
        row2: i32,
        column2: i32,
        reducer: crate::model::range_reduce::RangeReducer,
        acc: crate::model::range_reduce::RangeAgg,
    ) -> crate::model::range_reduce::RangeAgg {
        self.0
            .fold_range(sheet, row1, column1, row2, column2, reducer, acc)
    }

    pub(crate) fn evaluate_node_in_context(
        &mut self,
        node: &Node,
        cell: CellReferenceIndex,
    ) -> CalcResult {
        self.0.evaluate_node_in_context(node, cell)
    }

    pub(crate) fn evaluate_node_with_reference(
        &mut self,
        node: &Node,
        cell: CellReferenceIndex,
    ) -> CalcResult {
        self.0.evaluate_node_with_reference(node, cell)
    }

    /// Records `FormulaText`.
    pub(crate) fn formula_index_at(&mut self, sheet: u32, row: i32, column: i32) -> Option<i32> {
        self.0.formula_index_at(sheet, row, column)
    }

    /// Records `RowHidden`.
    pub(crate) fn row_hidden(&mut self, sheet: u32, row: i32) -> Result<bool, String> {
        self.0.row_hidden(sheet, row)
    }

    /// Records `SheetStructure`.
    pub(crate) fn sheet_dimension(&mut self, sheet: u32) -> Result<WorksheetDimension, String> {
        self.0.sheet_dimension(sheet)
    }

    /// Records `SheetStructure`.
    pub(crate) fn sheet_count(&mut self) -> usize {
        self.0.sheet_count()
    }

    /// Records `SheetStructure`.
    pub(crate) fn table_by_name(&mut self, name: &str) -> Option<&Table> {
        self.0.table_by_name(name)
    }

    /// Records `SheetStructure`.
    pub(crate) fn tables(&mut self) -> &HashMap<String, Table> {
        self.0.tables()
    }

    /// Records `FormulaText`.
    pub(crate) fn parsed_formula_node(&mut self, sheet: u32, index: i32) -> Option<&Node> {
        self.0
            .parsed_formulas
            .get(sheet as usize)?
            .get(index as usize)
            .map(|(node, _)| node)
    }

    /// A parsed defined name by exact (scope, lowercased name) key. Records
    /// `Name`, so `SHEET(a_name)` reads it the same way evaluating a
    /// `DefinedNameKind` node does.
    pub(crate) fn defined_name(
        &mut self,
        scope: Option<u32>,
        lowercased_name: &str,
    ) -> Option<&crate::model::ParsedDefinedName> {
        self.0.trace_input(Input::Name {
            name: lowercased_name.to_string(),
            scope,
        });
        self.0
            .parsed_defined_names
            .get(&(scope, lowercased_name.to_string()))
    }

    // -- workbook facts that are not cell state ---------------------------

    pub(crate) fn workbook_name(&self) -> &str {
        self.0.workbook_name()
    }

    pub(crate) fn worksheet_name(&mut self, sheet: u32) -> Result<String, String> {
        self.0.worksheet_name(sheet)
    }

    pub(crate) fn get_sheet_index_by_name(&self, name: &str) -> Option<u32> {
        self.0.get_sheet_index_by_name(name)
    }

    /// Records `FormulaText`.
    ///
    /// The recording lives here rather than at the call site so that reading a
    /// cell's formula-ness and recording the dependency on it are one act:
    /// `ISFORMULA` cannot read without recording, because there is no other
    /// way in.
    pub(crate) fn get_cell_formula(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Option<String>, String> {
        self.0.trace_input(Input::FormulaText((sheet, row, column)));
        self.0.get_cell_formula(sheet, row, column)
    }

    /// Records `FormulaText`. Same reasoning as [`Self::get_cell_formula`];
    /// this is `FORMULATEXT`'s way in.
    pub(crate) fn get_english_cell_formula(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Option<String>, String> {
        self.0.trace_input(Input::FormulaText((sheet, row, column)));
        self.0.get_english_cell_formula(sheet, row, column)
    }

    pub(crate) fn get_timezone(&self) -> String {
        self.0.get_timezone()
    }

    // -- evaluation-local state, not workbook state -----------------------

    pub(crate) fn locale(&self) -> &'m Locale {
        self.0.locale
    }

    pub(crate) fn tz(&self) -> &crate::tz::Tz {
        &self.0.tz
    }

    pub(crate) fn get_next_variable_id(&mut self) -> usize {
        self.0.get_next_variable_id()
    }

    pub(crate) fn get_next_lambda_id(&mut self) -> usize {
        self.0.get_next_lambda_id()
    }

    pub(crate) fn set_variable(&mut self, id: usize, value: CalcResult) {
        self.0.variable_stack.insert(id, value);
    }

    pub(crate) fn clear_variable(&mut self, id: usize) {
        self.0.variable_stack.remove(&id);
    }

    pub(crate) fn lambda(
        &self,
        id: usize,
    ) -> Option<&(Vec<crate::expressions::parser::NamedVariable>, Node)> {
        self.0.lambdas.get(&id)
    }

    pub(crate) fn set_lambda(
        &mut self,
        id: usize,
        lambda: (Vec<crate::expressions::parser::NamedVariable>, Node),
    ) {
        self.0.lambdas.insert(id, lambda);
    }

    /// A dynamic link produced by `HYPERLINK`. A write, not a read.
    pub(crate) fn set_link(&mut self, key: (u32, i32, i32), link: crate::types::Link) {
        self.0.links.insert(key, link);
    }
}
