//! Incremental recalculation: dependency-graph construction, the selective
//! evaluation pass built on it, and the changed-cell delta it exposes.
//!
//! A full pass rebuilds the forward dependency graph from a static walk of every
//! formula's AST; the incremental pass recomputes only the cells reachable from
//! the ones that changed and records which ones moved, so
//! [`Model::take_changed_cells`] can report a precise delta. See
//! [`crate::dependency_graph`] for the graph and modes.

use std::collections::{HashMap, HashSet};

use super::{CellOrRange, ChangedCells, ParsedDefinedName};
use crate::cell::CellValue;
use crate::dependency_graph::Position;
use crate::expressions::parser::Node;
use crate::expressions::types::CellReferenceIndex;
use crate::functions::Function;
use crate::model::Model;
use crate::types::{Cell, CellType};

/// Below this many formula cells, incremental never falls back on fanout: the
/// bookkeeping is cheap in absolute terms and full has no edge to exploit.
const INCREMENTAL_FANOUT_FLOOR: usize = 1024;

/// Fall back to a full pass once an edit's fanout reaches this fraction of the
/// formula cells: `2` means half, past which full is cheaper than the
/// incremental bookkeeping it would save.
const INCREMENTAL_FANOUT_RATIO: usize = 2;

/// A cell's observable signature for incremental change detection: value, type
/// (so an error and a same-text literal differ), and dynamic link.
type ChangeKey = (CellType, ChangeValue, Option<crate::types::Link>);

/// Every cell's full observable state (`ChangeKey` plus conditional format),
/// used by the `Verify` check to compare incremental against full.
#[cfg(feature = "recalc_verify")]
type RenderSnapshot = HashMap<Position, (Option<ChangeKey>, Vec<crate::cf_types::CfCellResult>)>;

/// A cell value flattened for change detection. Numbers are kept as bits so a
/// `+0.0`/`-0.0` flip is seen and a `NaN` does not report as changed forever.
#[derive(PartialEq, Debug)]
enum ChangeValue {
    None,
    Boolean(bool),
    Number(u64),
    String(String),
}

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

    /// Whether a formula reads a cell through a reference the static edges do not
    /// capture: `OFFSET`, `INDIRECT`, or the range operator with a computed
    /// endpoint. The frontier can recompute such a cell before the precedent it
    /// resolves to, reading a stale value, so an edit reaching one forces a full
    /// pass where evaluation order is fixed.
    fn node_has_dynamic_reference(
        &self,
        node: &Node,
        sheet: u32,
        seen_names: &mut HashSet<(String, Option<u32>)>,
    ) -> bool {
        match node {
            Node::FunctionKind { kind, args } => {
                matches!(kind, Function::Offset | Function::Indirect)
                    || args
                        .iter()
                        .any(|arg| self.node_has_dynamic_reference(arg, sheet, seen_names))
            }
            Node::NamedFunctionKind { name, args, id } => {
                if args
                    .iter()
                    .any(|arg| self.node_has_dynamic_reference(arg, sheet, seen_names))
                {
                    return true;
                }
                if id.is_none() {
                    if let Some((scope, body)) = self.resolve_named_lambda(name, sheet) {
                        if seen_names.insert((name.clone(), scope)) {
                            return self.node_has_dynamic_reference(&body, sheet, seen_names);
                        }
                    }
                }
                false
            }
            Node::LambdaCallKind { lambda, args } => {
                self.node_has_dynamic_reference(lambda, sheet, seen_names)
                    || args
                        .iter()
                        .any(|arg| self.node_has_dynamic_reference(arg, sheet, seen_names))
            }
            Node::LambdaDefKind { body, .. } => {
                self.node_has_dynamic_reference(body, sheet, seen_names)
            }
            Node::OpRangeKind { .. } => true,
            Node::OpConcatenateKind { left, right }
            | Node::OpSumKind { left, right, .. }
            | Node::OpProductKind { left, right, .. }
            | Node::OpPowerKind { left, right }
            | Node::CompareKind { left, right, .. } => {
                self.node_has_dynamic_reference(left, sheet, seen_names)
                    || self.node_has_dynamic_reference(right, sheet, seen_names)
            }
            Node::UnaryKind { right, .. } => {
                self.node_has_dynamic_reference(right, sheet, seen_names)
            }
            Node::ImplicitIntersection { child, .. } | Node::SpillRangeOperator { child } => {
                self.node_has_dynamic_reference(child, sheet, seen_names)
            }
            // A bare defined name resolves to a cell/range (captured by static
            // edges) or a lambda whose body can call `OFFSET`/`INDIRECT`; resolve
            // and walk it as the evaluator does, mirroring `collect_references`.
            Node::DefinedNameKind((name, scope, _)) => {
                if seen_names.insert((name.clone(), *scope)) {
                    if let Ok(Some(ParsedDefinedName::LambdaDefinition(_, body))) =
                        self.get_parsed_defined_name(name, *scope)
                    {
                        return self.node_has_dynamic_reference(&body, sheet, seen_names);
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
        // Cells reading a precedent through a dynamic reference the static edges
        // miss. They join `array_cells` so an edit reaching one forces a full
        // pass, the only way to recompute them after the precedent they resolve to.
        let mut dynamic_reference_cells = HashSet::new();
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
                        if self.node_has_dynamic_reference(node, sheet, &mut HashSet::new()) {
                            dynamic_reference_cells.insert((sheet, *row, *col));
                        }
                    }
                }
            }
        }
        self.volatile_cells = volatile_cells;
        self.array_cells.extend(dynamic_reference_cells);
        self.formula_cell_count = formula_cell_count;
    }

    /// Collects every cell and range a formula can statically read into `out`.
    /// Dynamic branches are over-approximated (all `IF` branches), a safe superset;
    /// truly dynamic references (`INDIRECT`, `OFFSET`, computed endpoints) are left
    /// to volatile handling. Resolves defined names and lambda bodies, cycle-guarded.
    fn collect_references(
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

    /// Runs the incremental pass, then a full pass, and asserts they agree, and
    /// that the recorded delta names every cell whose observable state moved.
    /// Backs [`RecalcMode::Verify`](crate::dependency_graph::RecalcMode::Verify).
    #[cfg(feature = "recalc_verify")]
    pub(crate) fn verify_incremental_matches_full(&mut self) {
        // Only meaningful when the run was actually incremental: a full fallback
        // has nothing to check, and a second full re-rolls volatiles into false diffs.
        let before = self.render_snapshot();
        // Arm delta tracking for this pass so the completeness check below runs on
        // every evaluate, not only after a consumer has called `take_changed_cells`.
        // Scoped to the verify build, so it never affects the shipped delta.
        if matches!(self.changed_cells, ChangedCells::All) {
            self.changed_cells = ChangedCells::Delta(HashSet::new());
        }
        if !self.evaluate_selective() {
            return;
        }
        let incremental = self.render_snapshot();
        // Volatiles (RAND, NOW) re-roll every pass and are always seeded into the
        // delta, so exclude them from both checks.
        let tainted = self
            .graph
            .reachable(self.volatile_cells.iter().copied().collect());
        // Delta completeness: every non-volatile cell whose observable state
        // (value, type, link, conditional format) moved must be in the delta.
        if let ChangedCells::Delta(delta) = &self.changed_cells {
            for position in before.keys().chain(incremental.keys()) {
                let changed = before.get(position) != incremental.get(position);
                assert!(
                    tainted.contains(position) || !changed || delta.contains(position),
                    "cell {position:?} changed but is missing from the delta"
                );
            }
        }
        // Value equivalence: incremental and full must agree on the same state.
        self.evaluate_full();
        let full = self.render_snapshot();
        let strip = |mut snapshot: RenderSnapshot| {
            snapshot.retain(|position, _| !tainted.contains(position));
            snapshot
        };
        assert_eq!(
            strip(incremental),
            strip(full),
            "incremental recalc diverged from full recompute"
        );
    }

    /// Every cell's full observable state (value/type/link + conditional format),
    /// for the delta-completeness check: a cell whose state moves must be in the
    /// delta.
    #[cfg(feature = "recalc_verify")]
    fn render_snapshot(&self) -> RenderSnapshot {
        let mut positions: HashSet<Position> = self.cf_cache.keys().copied().collect();
        for c in self.get_all_cells() {
            positions.insert((c.index, c.row, c.column));
        }
        positions
            .into_iter()
            .map(|p| {
                (
                    p,
                    (
                        self.change_key(p),
                        self.cf_cache.get(&p).cloned().unwrap_or_default(),
                    ),
                )
            })
            .collect()
    }

    /// Adds to the delta any cell whose conditional-format result moved between
    /// `cf_before` and the rebuilt `cf_cache`. CF has no dependency edges, so a
    /// value or CF-rule change can move a cell's format with no value change.
    fn record_cf_changes(
        &mut self,
        cf_before: HashMap<Position, Vec<crate::cf_types::CfCellResult>>,
    ) {
        if let ChangedCells::Delta(delta) = &mut self.changed_cells {
            for (position, results) in &self.cf_cache {
                if cf_before.get(position) != Some(results) {
                    delta.insert(*position);
                }
            }
            for position in cf_before.keys() {
                if !self.cf_cache.contains_key(position) {
                    delta.insert(*position);
                }
            }
        }
    }

    /// Recomputes only the cells reachable from the dirty set, returning `true`.
    /// Returns `false` after falling back to a full recompute for anything the
    /// incremental path cannot model.
    pub(crate) fn evaluate_selective(&mut self) -> bool {
        if self.graph.should_recompute_full() {
            // A full from a shape-changing edit or the first pass may change any
            // cell, so drop the delta. A redundant full with nothing pending is a
            // no-op and keeps the delta, unless volatiles are present: a full pass
            // re-rolls them, so their new values would be missed. Treat that as an
            // everything-changed delta too.
            if self.graph.full_reflects_change() || !self.volatile_cells.is_empty() {
                self.evaluate_full_untracked();
            } else {
                // A redundant full preserves the delta, but a conditional-format
                // edit moves CF results with no cell value change, so diff those.
                let cf_before = self.cf_cache.clone();
                self.evaluate_full();
                self.record_cf_changes(cf_before);
            }
            return false;
        }
        // Volatile cells re-roll on every full pass, so mark them dirty to
        // recompute them (and their dependents) on every incremental pass too.
        for &cell in &self.volatile_cells {
            self.graph.mark_dirty(cell);
        }
        let (seeds, affected) = self.graph.take_seeds_and_affected();
        // A wide-fanout edit reaches most of the workbook, where incremental
        // bookkeeping costs about as much as it saves; past half the formulas a
        // full pass is cheaper. The floor keeps small workbooks on the fast path.
        if self.formula_cell_count >= INCREMENTAL_FANOUT_FLOOR
            && affected.len() * INCREMENTAL_FANOUT_RATIO >= self.formula_cell_count
        {
            self.evaluate_full_untracked();
            return false;
        }
        // Array and spill cells need the full pass's two-phase ordering, so an
        // edit reaching one falls back to full; edits that do not stay incremental.
        if affected.iter().any(|cell| self.array_cells.contains(cell)) {
            self.evaluate_full_untracked();
            return false;
        }
        // Recompute the affected cells and collect the ones whose value actually
        // moved. A cycle in the affected set has no topological order, so fall
        // back to recomputing the whole set, where `evaluate_cell`'s recursion
        // still reports `#CIRC!`.
        let changed = match self.graph.topo_order(&affected) {
            Some(order) => self.recompute_frontier(affected, &seeds, order),
            None => self.recompute_all(affected, &seeds),
        };
        // Record only the changed cells for `take_changed_cells`, unless a full
        // pass has already marked everything changed since the last read.
        if let ChangedCells::Delta(delta) = &mut self.changed_cells {
            delta.extend(changed);
        }
        self.graph.after_incremental();
        let cf_before = self.cf_cache.clone();
        self.evaluate_conditional_formatting();
        self.record_cf_changes(cf_before);
        true
    }

    /// A cell's observable signature: value, type (so an error and a same-text
    /// literal differ), and dynamic link (a HYPERLINK target can move under a
    /// fixed label).
    fn change_key(&self, position @ (sheet, row, column): Position) -> Option<ChangeKey> {
        let value = match self.get_cell_value_by_index(sheet, row, column).ok()? {
            CellValue::None => ChangeValue::None,
            CellValue::Boolean(b) => ChangeValue::Boolean(b),
            // By bits, so a +0.0/-0.0 flip is seen and NaN does not report forever.
            CellValue::Number(n) => ChangeValue::Number(n.to_bits()),
            CellValue::String(s) => ChangeValue::String(s),
        };
        let cell_type = self.get_cell_type(sheet, row, column).ok()?;
        Some((cell_type, value, self.links.get(&position).cloned()))
    }

    /// Clears a cell's cached state so the next `evaluate_cell` recomputes it.
    /// Drops the dynamic link too, as a full pass would, so a cell that no longer
    /// resolves to a `HYPERLINK` does not keep a stale one.
    fn invalidate(&mut self, position: Position) {
        self.cells.remove(&position);
        self.links.remove(&position);
    }

    /// Recomputes the affected set in topological order, propagating to a cell's
    /// dependents only when its value moved. Seeds are the edited cells, so they
    /// always count as changed and propagate; an unchanged non-seed stops the
    /// fanout there. Returns the changed cells.
    fn recompute_frontier(
        &mut self,
        affected: HashSet<Position>,
        seeds: &[Position],
        order: Vec<Position>,
    ) -> Vec<Position> {
        self.recompute_scope = Some(affected);
        let seeded: HashSet<Position> = seeds.iter().copied().collect();
        let mut stale = seeded.clone();
        let mut changed = Vec::new();
        for position in order {
            if !stale.contains(&position) {
                continue;
            }
            let before = self.change_key(position);
            self.invalidate(position);
            let (sheet, row, column) = position;
            self.evaluate_cell(CellReferenceIndex { sheet, row, column });
            if seeded.contains(&position) || self.change_key(position) != before {
                changed.push(position);
                stale.extend(self.graph.dependents_of(position));
            }
        }
        self.recompute_scope = None;
        changed
    }

    /// Recomputes the whole affected set, used when a cycle prevents ordering.
    /// Returns the seeds plus every other cell whose value moved.
    fn recompute_all(&mut self, affected: HashSet<Position>, seeds: &[Position]) -> Vec<Position> {
        let mut order: Vec<Position> = affected.iter().copied().collect();
        order.sort_unstable();
        let before: HashMap<Position, Option<ChangeKey>> =
            order.iter().map(|&p| (p, self.change_key(p))).collect();
        for &position in &order {
            self.invalidate(position);
        }
        self.recompute_scope = Some(affected);
        for &(sheet, row, column) in &order {
            self.evaluate_cell(CellReferenceIndex { sheet, row, column });
        }
        self.recompute_scope = None;
        let seeded: HashSet<Position> = seeds.iter().copied().collect();
        order
            .into_iter()
            .filter(|p| seeded.contains(p) || self.change_key(*p) != before[p])
            .collect()
    }

    /// Full recompute whose result is not expressible as a delta: it may have
    /// changed any cell, so the change record is cleared and the next
    /// `take_changed_cells` reports "no delta" until incremental passes rebuild it.
    pub(crate) fn evaluate_full_untracked(&mut self) {
        self.evaluate_full();
        self.changed_cells = ChangedCells::All;
    }

    /// Returns the cells recomputed by incremental evaluations since the last
    /// call, sorted, and clears the record. `None` means a full recompute has run,
    /// so every cell should be treated as potentially changed.
    pub fn take_changed_cells(&mut self) -> Option<Vec<CellReferenceIndex>> {
        // Reading re-arms tracking: the record resets to an empty delta, so
        // subsequent incremental passes accumulate afresh.
        let taken = std::mem::replace(&mut self.changed_cells, ChangedCells::Delta(HashSet::new()));
        let ChangedCells::Delta(cells) = taken else {
            return None;
        };
        let mut cells: Vec<Position> = cells.into_iter().collect();
        cells.sort_unstable();
        Some(
            cells
                .into_iter()
                .map(|(sheet, row, column)| CellReferenceIndex { sheet, row, column })
                .collect(),
        )
    }
}
