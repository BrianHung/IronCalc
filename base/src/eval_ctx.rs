//! The receiver formula evaluation runs on.
//!
//! `EvalCtx` is a newtype around `&mut Model` whose inner reference is private
//! to this module. `functions/` — and the evaluation support in `cast.rs` and
//! `arithmetic.rs` — take `EvalCtx` rather than `Model`, so the only workbook
//! state they can reach is what is re-exposed below.
//!
//! This is what makes the boundary constructional rather than conventional: a
//! function implementation cannot take an undeclared dependency on workbook
//! internals, because `self.workbook` does not resolve in `functions/` — the
//! field belongs to `Model`, and `functions/` never holds one.
//!
//! Privacy alone could not do this. `xlsx` and the bindings are separate
//! crates that legitimately read `model.workbook`, so `Model::workbook` and
//! the getters reaching it must stay `pub` — and `functions/` is in the same
//! crate, so it could call them too. Rust visibility cannot say "everyone
//! except this sibling module"; a receiver type can.
//!
//! Every method here is a pass-through. The set is exactly what evaluation
//! calls today: adding to it is a deliberate act, visible in review.

use std::collections::HashMap;

use crate::{
    calc_result::CalcResult,
    expressions::{
        parser::{ArrayNode, NamedVariable, Node},
        types::CellReferenceIndex,
    },
    locale::Locale,
    model::{Model, ParsedDefinedName},
    types::{Link, Table},
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
    // -- reads of cell state ----------------------------------------------

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

    pub(crate) fn formula_index_at(&self, sheet: u32, row: i32, column: i32) -> Option<i32> {
        self.0.formula_index_at(sheet, row, column)
    }

    pub(crate) fn parsed_formula_node(&self, sheet: u32, index: i32) -> Option<&Node> {
        self.0.parsed_formula_node(sheet, index)
    }

    pub(crate) fn get_cell_formula(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Option<String>, String> {
        self.0.get_cell_formula(sheet, row, column)
    }

    pub(crate) fn get_english_cell_formula(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Option<String>, String> {
        self.0.get_english_cell_formula(sheet, row, column)
    }

    // -- sheet structure --------------------------------------------------

    pub(crate) fn sheet_dimension(&self, sheet: u32) -> Result<WorksheetDimension, String> {
        self.0.sheet_dimension(sheet)
    }

    pub(crate) fn row_hidden(&self, sheet: u32, row: i32) -> Result<bool, String> {
        self.0.row_hidden(sheet, row)
    }

    pub(crate) fn sheet_count(&self) -> usize {
        self.0.sheet_count()
    }

    pub(crate) fn tables(&self) -> &HashMap<String, Table> {
        self.0.tables()
    }

    pub(crate) fn workbook_name(&self) -> &str {
        self.0.workbook_name()
    }

    pub(crate) fn worksheet_name(&self, sheet: u32) -> Result<String, String> {
        self.0.worksheet_name(sheet)
    }

    pub(crate) fn get_sheet_index_by_name(&self, name: &str) -> Option<u32> {
        self.0.get_sheet_index_by_name(name)
    }

    /// A parsed defined name by exact (scope, lowercased name) key.
    pub(crate) fn defined_name(
        &self,
        scope: Option<u32>,
        lowercased_name: &str,
    ) -> Option<&ParsedDefinedName> {
        self.0.defined_name(scope, lowercased_name)
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

    pub(crate) fn lambda(&self, id: usize) -> Option<&(Vec<NamedVariable>, Node)> {
        self.0.lambdas.get(&id)
    }

    pub(crate) fn set_lambda(&mut self, id: usize, lambda: (Vec<NamedVariable>, Node)) {
        self.0.lambdas.insert(id, lambda);
    }

    /// A dynamic link produced by `HYPERLINK`. A write, not a read.
    pub(crate) fn set_link(&mut self, key: (u32, i32, i32), link: Link) {
        self.0.links.insert(key, link);
    }
}
