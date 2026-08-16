//! Incremental recalculation: dependency-graph construction and the selective
//! evaluation pass built on it.
//!
//! A full pass rebuilds the forward dependency graph from a static walk of every
//! formula's AST; the incremental pass recomputes only the cells reachable from
//! the ones that changed. See [`crate::dependency_graph`] for the graph and modes.

use std::collections::HashSet;

use super::{CellOrRange, ParsedDefinedName};
#[cfg(feature = "recalc_verify")]
use crate::cell::CellValue;
use crate::dependency_graph::Position;
use crate::expressions::parser::Node;
use crate::expressions::types::CellReferenceIndex;
use crate::functions::Function;
use crate::model::Model;
use crate::types::Cell;
#[cfg(feature = "recalc_verify")]
use std::collections::HashMap;

/// Below this many formula cells, incremental never falls back on fanout: the
/// bookkeeping is cheap in absolute terms and full has no edge to exploit.
const INCREMENTAL_FANOUT_FLOOR: usize = 1024;

/// Fall back to a full pass once an edit's fanout reaches this fraction of the
/// formula cells: `2` means half, past which full is cheaper than the
/// incremental bookkeeping it would save.
const INCREMENTAL_FANOUT_RATIO: usize = 2;

/// Functions whose result can change without any input changing: random and
/// clock functions, and the reference functions whose target our static edges do
/// not capture. A cell calling one must be recomputed on every pass.
// Add new volatile functions here, or their cells will not refresh incrementally.
fn is_volatile_function(kind: &Function) -> bool {
    matches!(
        kind,
        Function::Rand
            | Function::Randbetween
            | Function::Randarray
            | Function::Now
            | Function::Today
            | Function::Offset
            | Function::Indirect
            | Function::Cell
            | Function::Info
    )
}

impl Model<'_> {
    /// Records the positions of array and spill cells after a full pass, so the
    /// incremental path can fall back to full for any edit that reaches one. Must
    /// run after spilling, when the spill output cells exist.
    pub(crate) fn collect_array_cells(&mut self) {
        let mut array_cells = HashSet::new();
        for (sheet_index, worksheet) in self.workbook.worksheets.iter().enumerate() {
            let sheet = sheet_index as u32;
            for (row, row_data) in &worksheet.sheet_data {
                for (col, cell) in row_data {
                    if matches!(cell, Cell::ArrayFormula { .. } | Cell::SpillCell { .. }) {
                        array_cells.insert((sheet, *row, *col));
                    }
                }
            }
        }
        self.array_cells = array_cells;
    }

    /// Whether a formula tree calls a volatile function anywhere. Exhaustive so a
    /// new `Node` variant must be classified here rather than read as non-volatile.
    /// Defined-name lambdas run their body, so resolve and walk it, cycle-guarded.
    fn node_is_volatile(
        &self,
        node: &Node,
        sheet: u32,
        seen_names: &mut HashSet<(String, Option<u32>)>,
    ) -> bool {
        match node {
            Node::FunctionKind { kind, args } => {
                is_volatile_function(kind)
                    || args
                        .iter()
                        .any(|arg| self.node_is_volatile(arg, sheet, seen_names))
            }
            Node::NamedFunctionKind { name, args, id } => {
                if args
                    .iter()
                    .any(|arg| self.node_is_volatile(arg, sheet, seen_names))
                {
                    return true;
                }
                if id.is_none() {
                    if let Some((scope, body)) = self.resolve_named_lambda(name, sheet) {
                        if seen_names.insert((name.clone(), scope)) {
                            return self.node_is_volatile(&body, sheet, seen_names);
                        }
                    }
                }
                false
            }
            Node::LambdaCallKind { lambda, args } => {
                self.node_is_volatile(lambda, sheet, seen_names)
                    || args
                        .iter()
                        .any(|arg| self.node_is_volatile(arg, sheet, seen_names))
            }
            Node::LambdaDefKind { body, .. } => self.node_is_volatile(body, sheet, seen_names),
            // The range operator survives parsing only with a dynamic endpoint
            // (static `A1:A10` folds to one reference), so its span is volatile.
            Node::OpRangeKind { .. } => true,
            Node::OpConcatenateKind { left, right }
            | Node::OpSumKind { left, right, .. }
            | Node::OpProductKind { left, right, .. }
            | Node::OpPowerKind { left, right }
            | Node::CompareKind { left, right, .. } => {
                self.node_is_volatile(left, sheet, seen_names)
                    || self.node_is_volatile(right, sheet, seen_names)
            }
            Node::UnaryKind { right, .. } => self.node_is_volatile(right, sheet, seen_names),
            Node::ImplicitIntersection { child, .. } | Node::SpillRangeOperator { child } => {
                self.node_is_volatile(child, sheet, seen_names)
            }
            // A bare defined name resolves to a cell/range (inert) or a lambda
            // whose body can call a volatile function; resolve and walk it as the
            // evaluator does, mirroring `collect_references`.
            Node::DefinedNameKind((name, scope, _)) => {
                if seen_names.insert((name.clone(), *scope)) {
                    if let Ok(Some(ParsedDefinedName::LambdaDefinition(_, body))) =
                        self.get_parsed_defined_name(name, *scope)
                    {
                        return self.node_is_volatile(&body, sheet, seen_names);
                    }
                }
                false
            }
            Node::BooleanKind(_)
            | Node::NumberKind(_)
            | Node::StringKind(_)
            | Node::ReferenceKind { .. }
            | Node::RangeKind { .. }
            | Node::WrongReferenceKind { .. }
            | Node::WrongRangeKind { .. }
            | Node::ArrayKind(_)
            | Node::TableNameKind(_)
            | Node::NamedVariableKind { .. }
            | Node::ErrorKind(_)
            | Node::ParseErrorKind { .. }
            | Node::EmptyArgKind => false,
        }
    }

    /// Records the cells whose formula calls a volatile function. A full pass
    /// re-rolls these on every evaluation; recording them lets the incremental
    /// path recompute them on every edit so it matches that behavior.
    pub(crate) fn collect_volatile_cells(&mut self) {
        let mut volatile_cells = HashSet::new();
        let mut formula_cell_count = 0;
        for (sheet_index, worksheet) in self.workbook.worksheets.iter().enumerate() {
            let sheet = sheet_index as u32;
            for (row, row_data) in &worksheet.sheet_data {
                for (col, cell) in row_data {
                    if let Some(formula) = cell.get_formula() {
                        formula_cell_count += 1;
                        let node = &self.parsed_formulas[sheet as usize][formula as usize].0;
                        if self.node_is_volatile(node, sheet, &mut HashSet::new()) {
                            volatile_cells.insert((sheet, *row, *col));
                        }
                    }
                }
            }
        }
        self.volatile_cells = volatile_cells;
        self.formula_cell_count = formula_cell_count;
    }

    /// Collects every cell and range a formula can statically read into `out`.
    /// Dynamic branches are over-approximated (all `IF` branches), a safe superset;
    /// truly dynamic references (`INDIRECT`, `OFFSET`, computed endpoints) are left
    /// to volatile handling. Resolves defined names and lambda bodies, cycle-guarded.
    pub(in crate::model) fn collect_references(
        &self,
        node: &Node,
        context: CellReferenceIndex,
        out: &mut Vec<CellOrRange>,
        seen_names: &mut HashSet<(String, Option<u32>)>,
    ) {
        let absolute_coord = |absolute: bool, value: i32, offset: i32| {
            if absolute {
                value
            } else {
                value + offset
            }
        };
        match node {
            Node::ReferenceKind {
                sheet_index,
                absolute_row,
                absolute_column,
                row,
                column,
                ..
            } => {
                let r = absolute_coord(*absolute_row, *row, context.row);
                let c = absolute_coord(*absolute_column, *column, context.column);
                out.push(CellOrRange::Cell((*sheet_index, r, c)));
            }
            Node::RangeKind {
                sheet_index,
                absolute_row1,
                absolute_column1,
                row1,
                column1,
                absolute_row2,
                absolute_column2,
                row2,
                column2,
                ..
            } => {
                let r1 = absolute_coord(*absolute_row1, *row1, context.row);
                let c1 = absolute_coord(*absolute_column1, *column1, context.column);
                let r2 = absolute_coord(*absolute_row2, *row2, context.row);
                let c2 = absolute_coord(*absolute_column2, *column2, context.column);
                out.push(CellOrRange::Range((
                    *sheet_index,
                    r1.min(r2),
                    c1.min(c2),
                    r1.max(r2),
                    c1.max(c2),
                )));
            }
            Node::DefinedNameKind((name, scope, _)) => {
                if seen_names.insert((name.clone(), *scope)) {
                    if let Ok(Some(parsed)) = self.get_parsed_defined_name(name, *scope) {
                        match parsed {
                            ParsedDefinedName::CellReference(r) => {
                                out.push(CellOrRange::Cell((r.sheet, r.row, r.column)));
                            }
                            ParsedDefinedName::RangeReference(range) => {
                                out.push(CellOrRange::Range((
                                    range.left.sheet,
                                    range.left.row.min(range.right.row),
                                    range.left.column.min(range.right.column),
                                    range.left.row.max(range.right.row),
                                    range.left.column.max(range.right.column),
                                )));
                            }
                            ParsedDefinedName::LambdaDefinition(_, body) => {
                                self.collect_references(&body, context, out, seen_names);
                            }
                            ParsedDefinedName::InvalidDefinedNameFormula => {}
                        }
                    }
                }
            }
            Node::FunctionKind { args, .. } => {
                for arg in args {
                    self.collect_references(arg, context, out, seen_names);
                }
            }
            Node::NamedFunctionKind { name, args, id } => {
                for arg in args {
                    self.collect_references(arg, context, out, seen_names);
                }
                // A defined-name lambda (`id: None`) runs its body, which can read
                // cells beyond its args; resolve and walk it as the evaluator does.
                if id.is_none() {
                    if let Some((scope, body)) = self.resolve_named_lambda(name, context.sheet) {
                        if seen_names.insert((name.clone(), scope)) {
                            self.collect_references(&body, context, out, seen_names);
                        }
                    }
                }
            }
            Node::LambdaCallKind { lambda, args } => {
                self.collect_references(lambda, context, out, seen_names);
                for arg in args {
                    self.collect_references(arg, context, out, seen_names);
                }
            }
            Node::LambdaDefKind { body, .. } => {
                self.collect_references(body, context, out, seen_names)
            }
            Node::OpRangeKind { left, right }
            | Node::OpConcatenateKind { left, right }
            | Node::OpSumKind { left, right, .. }
            | Node::OpProductKind { left, right, .. }
            | Node::OpPowerKind { left, right }
            | Node::CompareKind { left, right, .. } => {
                self.collect_references(left, context, out, seen_names);
                self.collect_references(right, context, out, seen_names);
            }
            Node::UnaryKind { right, .. } => {
                self.collect_references(right, context, out, seen_names)
            }
            Node::ImplicitIntersection { child, .. } | Node::SpillRangeOperator { child } => {
                self.collect_references(child, context, out, seen_names);
            }
            Node::BooleanKind(_)
            | Node::NumberKind(_)
            | Node::StringKind(_)
            | Node::WrongReferenceKind { .. }
            | Node::WrongRangeKind { .. }
            | Node::ArrayKind(_)
            | Node::TableNameKind(_)
            | Node::NamedVariableKind { .. }
            | Node::ErrorKind(_)
            | Node::ParseErrorKind { .. }
            | Node::EmptyArgKind => {}
        }
    }

    /// Rebuilds the forward graph from the parsed formulas: an edge from each cell
    /// or range a formula reads. Runs on a full pass; derived from formula
    /// structure, so it is stable across value edits and rebuilt only on full.
    pub(crate) fn build_dependency_graph(&mut self) {
        self.graph.clear_edges();
        let mut edges: Vec<(Position, Vec<CellOrRange>)> = Vec::new();
        for (sheet_index, worksheet) in self.workbook.worksheets.iter().enumerate() {
            let sheet = sheet_index as u32;
            for (row, row_data) in &worksheet.sheet_data {
                for (col, cell) in row_data {
                    if let Some(formula) = cell.get_formula() {
                        let dependent = CellReferenceIndex {
                            sheet,
                            row: *row,
                            column: *col,
                        };
                        let node = &self.parsed_formulas[sheet as usize][formula as usize].0;
                        let mut refs = Vec::new();
                        self.collect_references(node, dependent, &mut refs, &mut HashSet::new());
                        edges.push(((sheet, *row, *col), refs));
                    }
                }
            }
        }
        for (dependent, refs) in edges {
            for reference in refs {
                match reference {
                    CellOrRange::Cell(precedent) => self.graph.add_cell_edge(precedent, dependent),
                    CellOrRange::Range(area) => self.graph.add_range_edge(area, dependent),
                }
            }
        }
    }

    /// Runs the incremental pass, then a full pass, and asserts they agree.
    /// Backs [`RecalcMode::Verify`](crate::dependency_graph::RecalcMode::Verify).
    #[cfg(feature = "recalc_verify")]
    pub(crate) fn verify_incremental_matches_full(&mut self) {
        // Only meaningful when the run was actually incremental: a full fallback
        // has nothing to check, and a second full re-rolls volatiles into false diffs.
        if !self.evaluate_selective() {
            return;
        }
        let incremental = self.snapshot_values();
        self.evaluate_full();
        let full = self.snapshot_values();
        // Volatiles (RAND, NOW) and their dependents re-roll each pass, so they
        // legitimately differ; drop them and require the rest to match.
        let tainted = self
            .graph
            .reachable(self.volatile_cells.iter().copied().collect());
        let strip = |mut values: HashMap<Position, CellValue>| {
            values.retain(|position, _| !tainted.contains(position));
            values
        };
        assert_eq!(
            strip(incremental),
            strip(full),
            "incremental recalc diverged from full recompute"
        );
    }

    /// Snapshot of every populated cell's value, keyed by position. Used by
    /// `RecalcMode::Verify` to diff the incremental result against the full one.
    #[cfg(feature = "recalc_verify")]
    fn snapshot_values(&self) -> HashMap<Position, CellValue> {
        self.get_all_cells()
            .into_iter()
            .filter_map(|c| {
                self.get_cell_value_by_index(c.index, c.row, c.column)
                    .ok()
                    .map(|value| ((c.index, c.row, c.column), value))
            })
            .collect()
    }

    /// Recomputes only the cells reachable from the dirty set, returning `true`.
    /// Returns `false` after falling back to a full recompute for anything the
    /// incremental path cannot model.
    pub(crate) fn evaluate_selective(&mut self) -> bool {
        // Range reductions memoized in a prior pass summarize stale values.
        self.range_reduce_cache.clear();
        if self.graph.should_recompute_full() {
            self.evaluate_full();
            return false;
        }
        // Volatile cells re-roll on every full pass, so mark them dirty to
        // recompute them (and their dependents) on every incremental pass too.
        for &cell in &self.volatile_cells {
            self.graph.mark_dirty(cell);
        }
        let affected = self.graph.take_affected();
        // A wide-fanout edit reaches most of the workbook, where incremental
        // bookkeeping costs about as much as it saves; past half the formulas a
        // full pass is cheaper. The floor keeps small workbooks on the fast path.
        if self.formula_cell_count >= INCREMENTAL_FANOUT_FLOOR
            && affected.len() * INCREMENTAL_FANOUT_RATIO >= self.formula_cell_count
        {
            self.evaluate_full();
            return false;
        }
        // Array and spill cells need the full pass's two-phase ordering, so an
        // edit reaching one falls back to full; edits that do not stay incremental.
        if affected.iter().any(|cell| self.array_cells.contains(cell)) {
            self.evaluate_full();
            return false;
        }
        // Recompute in the full pass's (sheet, row, column) order so a chain is
        // walked precedent-first and `evaluate_cell`'s recursion stays shallow.
        let mut to_recompute: Vec<Position> = affected.iter().copied().collect();
        to_recompute.sort_unstable();
        // Recompute only the affected cells; others keep their value and status.
        // Drop stale dynamic links too, as a full pass would.
        for position in &to_recompute {
            self.cells.remove(position);
            self.links.remove(position);
        }
        self.recompute_scope = Some(affected);
        for &(sheet, row, column) in &to_recompute {
            self.evaluate_cell(CellReferenceIndex { sheet, row, column });
        }
        self.recompute_scope = None;
        self.graph.after_incremental();
        self.evaluate_conditional_formatting();
        true
    }
}
