//! Incremental recalculation: dependency-graph construction, the selective
//! evaluation pass built on it, and the changed-cell delta it exposes.
//!
//! A full pass rebuilds the forward dependency graph from a static walk of every
//! formula's AST; the incremental pass recomputes only the cells reachable from
//! the ones that changed and records which ones moved, so
//! [`Model::take_changed_cells`] can report a precise delta. See
//! [`crate::dependency_graph`] for the graph and modes.

use std::collections::{HashMap, HashSet};

use super::{CellOrRange, ChangedCells, ChangedSinceRead, ParsedDefinedName};
use crate::cell::CellValue;
use crate::dependency_graph::Position;
#[cfg(feature = "recalc_verify")]
use crate::dependency_graph::RecalcMode;
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

/// Values that are not a function of the sheet. Incremental then full will
/// disagree even when both paths are correct, so Verify strips only this cone.
/// `OFFSET` is deterministic and is not stripped. A top-level `INDIRECT` is a
/// 1×1 dynamic array (Full, not compared). `SUM(INDIRECT(...))` stays Incremental.
fn is_nondeterministic_function(kind: &Function) -> bool {
    matches!(
        kind,
        Function::Rand
            | Function::Randbetween
            | Function::Randarray
            | Function::Now
            | Function::Today
    )
}

fn is_dynamic_ref_function(kind: &Function) -> bool {
    matches!(kind, Function::Offset | Function::Indirect)
}

/// `A1:expr` at any depth. A root-only check misses `=SUM((A1):(A10))`.
fn node_has_op_range(node: &Node) -> bool {
    match node {
        Node::OpRangeKind { .. } => true,
        Node::FunctionKind { args, .. } | Node::NamedFunctionKind { args, .. } => {
            args.iter().any(node_has_op_range)
        }
        Node::LambdaCallKind { lambda, args } => {
            node_has_op_range(lambda) || args.iter().any(node_has_op_range)
        }
        Node::LambdaDefKind { body, .. } => node_has_op_range(body),
        Node::OpConcatenateKind { left, right }
        | Node::OpSumKind { left, right, .. }
        | Node::OpProductKind { left, right, .. }
        | Node::OpPowerKind { left, right }
        | Node::CompareKind { left, right, .. } => {
            node_has_op_range(left) || node_has_op_range(right)
        }
        Node::UnaryKind { right, .. } => node_has_op_range(right),
        Node::ImplicitIntersection { child, .. } | Node::SpillRangeOperator { child } => {
            node_has_op_range(child)
        }
        _ => false,
    }
}

/// Whether an evaluate stayed incremental or fell back to a full pass.
pub(crate) enum EvalPass {
    Incremental,
    Full,
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
        self.graph.replace_arrays(array_cells);
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
        // A surviving `A1:expr` has a dynamic endpoint and is volatile even
        // when `expr` itself is not a volatile function. Check every depth:
        // `=SUM(A1:name)` nests `OpRangeKind` under `SUM`.
        node_has_op_range(node)
            || self.node_matches_function(node, sheet, seen_names, is_volatile_function)
    }

    fn node_is_nondeterministic(
        &self,
        node: &Node,
        sheet: u32,
        seen_names: &mut HashSet<(String, Option<u32>)>,
    ) -> bool {
        self.node_matches_function(node, sheet, seen_names, is_nondeterministic_function)
    }

    fn node_matches_function(
        &self,
        node: &Node,
        sheet: u32,
        seen_names: &mut HashSet<(String, Option<u32>)>,
        pred: fn(&Function) -> bool,
    ) -> bool {
        match node {
            Node::FunctionKind { kind, args } => {
                pred(kind)
                    || args
                        .iter()
                        .any(|arg| self.node_matches_function(arg, sheet, seen_names, pred))
            }
            Node::NamedFunctionKind { name, args, id } => {
                if args
                    .iter()
                    .any(|arg| self.node_matches_function(arg, sheet, seen_names, pred))
                {
                    return true;
                }
                if id.is_none() {
                    if let Some((scope, body)) = self.resolve_named_lambda(name, sheet) {
                        if seen_names.insert((name.clone(), scope)) {
                            return self.node_matches_function(&body, sheet, seen_names, pred);
                        }
                    }
                }
                false
            }
            Node::LambdaCallKind { lambda, args } => {
                self.node_matches_function(lambda, sheet, seen_names, pred)
                    || args
                        .iter()
                        .any(|arg| self.node_matches_function(arg, sheet, seen_names, pred))
            }
            Node::LambdaDefKind { body, .. } => {
                self.node_matches_function(body, sheet, seen_names, pred)
            }
            Node::OpRangeKind { left, right }
            | Node::OpConcatenateKind { left, right }
            | Node::OpSumKind { left, right, .. }
            | Node::OpProductKind { left, right, .. }
            | Node::OpPowerKind { left, right }
            | Node::CompareKind { left, right, .. } => {
                self.node_matches_function(left, sheet, seen_names, pred)
                    || self.node_matches_function(right, sheet, seen_names, pred)
            }
            Node::UnaryKind { right, .. } => {
                self.node_matches_function(right, sheet, seen_names, pred)
            }
            Node::ImplicitIntersection { child, .. } | Node::SpillRangeOperator { child } => {
                self.node_matches_function(child, sheet, seen_names, pred)
            }
            Node::DefinedNameKind((name, scope, _)) => {
                if seen_names.insert((name.clone(), *scope)) {
                    if let Ok(Some(ParsedDefinedName::LambdaDefinition(_, body))) =
                        self.get_parsed_defined_name(name, *scope)
                    {
                        return self.node_matches_function(&body, sheet, seen_names, pred);
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

    fn node_has_dynamic_reference(
        &self,
        node: &Node,
        sheet: u32,
        seen_names: &mut HashSet<(String, Option<u32>)>,
    ) -> bool {
        node_has_op_range(node)
            || self.node_matches_function(node, sheet, seen_names, is_dynamic_ref_function)
    }

    /// Records volatile formulas. RAND/NOW/TODAY re-roll every pass; OFFSET
    /// recomputes because static edges miss its target. Incremental marks
    /// RAND/NOW/TODAY dirty each pass. OFFSET/INDIRECT are skipped at
    /// `mark_dirty` and run in phase 2 instead.
    pub(crate) fn collect_volatile_cells(&mut self) {
        let mut volatile_cells = HashSet::new();
        // Cells reading a precedent through a dynamic reference the static edges
        // miss. Stored on their own type so they cannot be treated as arrays.
        let mut dynamic_reference_cells = HashSet::new();
        let mut nondeterministic_cells = HashSet::new();
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
                        if self.node_is_nondeterministic(node, sheet, &mut HashSet::new()) {
                            nondeterministic_cells.insert((sheet, *row, *col));
                        }
                    }
                }
            }
        }
        self.graph.replace_volatile(volatile_cells);
        self.graph.replace_dynamic_refs(dynamic_reference_cells);
        self.graph.replace_nondeterministic(nondeterministic_cells);
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

    /// When the pass stayed Incremental, runs a full pass and asserts they agree,
    /// and that the recorded delta names every cell whose observable state moved.
    /// A Full fallback has nothing to compare. Backs
    /// [`RecalcMode::Verify`](crate::dependency_graph::RecalcMode::Verify).
    #[cfg(feature = "recalc_verify")]
    pub(crate) fn verify_incremental_matches_full(&mut self) {
        let before = self.render_snapshot();
        // Completeness must run even if no consumer called take_changed_cells;
        // a fresh delta so a miss on this pass cannot hide behind an earlier one.
        let consumer =
            std::mem::replace(&mut self.changed_cells, ChangedCells::Delta(HashSet::new()));
        // Capture before evaluate: after_pass clears dirty. User edits plus
        // RAND/NOW/TODAY (seeded each Incremental pass). OFFSET is not a seed.
        let mut seeds = self.graph.pending_dirty();
        for cell in self.graph.volatile.iter() {
            if !self.graph.dynamic_refs.contains(&cell) {
                seeds.insert(cell);
            }
        }
        let pass = self.evaluate_selective();
        let this_pass =
            std::mem::replace(&mut self.changed_cells, ChangedCells::Delta(HashSet::new()));
        if matches!(pass, EvalPass::Incremental) {
            let incremental = self.render_snapshot();
            // RAND/NOW/TODAY re-roll. OFFSET stays in the compare when Incremental.
            // A top-level INDIRECT is a 1×1 dynamic array (Full, skipped).
            // SUM/PRODUCT(INDIRECT) stay Incremental and are compared.
            let tainted = self
                .graph
                .reachable(self.graph.nondeterministic.iter().collect());
            if let ChangedCells::Delta(delta) = &this_pass {
                for position in before.keys().chain(incremental.keys()) {
                    let changed = before.get(position) != incremental.get(position);
                    assert!(
                        tainted.contains(position) || !changed || delta.contains(position),
                        "cell {position:?} changed but is missing from the delta"
                    );
                }
                // Soundness: delta ⊆ (moved ∪ user/RAND seeds ∪ RAND cone).
                // `_set` writes the new value before evaluate, so a seed's
                // snapshot may not move even though the API reports it.
                // OFFSET is not a seed; it must not appear unless it moved.
                for position in delta {
                    let changed = before.get(position) != incremental.get(position);
                    assert!(
                        tainted.contains(position) || changed || seeds.contains(position),
                        "cell {position:?} is in the delta but did not change"
                    );
                }
            }
            self.changed_cells = merge_changed_cells(consumer, this_pass);
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
        } else {
            // A full fallback has nothing to compare; a second full re-rolls RAND/NOW.
            // Still restore the consumer delta so a redundant evaluate is not a miss.
            self.changed_cells = merge_changed_cells(consumer, this_pass);
        }
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
        positions.extend(self.links.keys().copied());
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

    pub(crate) fn evaluate_selective(&mut self) -> EvalPass {
        if self.graph.should_recompute_full() {
            // A full from a shape-changing edit or the first pass may change any
            // cell, so drop the delta. A redundant full with nothing pending is a
            // no-op and keeps the delta, unless RAND/NOW/TODAY are present: a
            // full pass re-rolls those, so treat that as Everything. OFFSET does
            // not re-roll and must not wipe the delta.
            if self.graph.full_reflects_change()
                || self.graph.nondeterministic.iter().next().is_some()
            {
                self.evaluate_full_untracked();
            } else {
                // A redundant full preserves the delta, but a conditional-format
                // edit moves CF results with no cell value change, so diff those.
                let cf_before = self.cf_cache.clone();
                self.evaluate_full();
                self.record_cf_changes(cf_before);
            }
            return EvalPass::Full;
        }
        // RAND/NOW/TODAY re-roll, so seed them each Incremental pass. OFFSET
        // and INDIRECT are also volatile but do not re-roll; they run after the
        // static frontier so they do not read a stale target.
        let volatiles: Vec<Position> = self.graph.volatile.iter().collect();
        for cell in volatiles {
            if !self.graph.dynamic_refs.contains(&cell) {
                self.graph.mark_dirty(cell);
            }
        }
        let (seeds, affected) = self.graph.take_seeds_and_affected();
        let dyn_seeds: Vec<Position> = self.graph.dynamic_refs.iter().collect();
        let dyn_cone = self.graph.reachable(dyn_seeds.clone());
        // A wide-fanout edit reaches most of the workbook, where incremental
        // bookkeeping costs about as much as it saves; past half the formulas a
        // full pass is cheaper. The floor keeps small workbooks on the fast path.
        // Verify skips this: it is a performance fallback, not a correctness one.
        if self.should_fallback_fanout(affected.len().max(dyn_cone.len())) {
            self.evaluate_full_untracked();
            return EvalPass::Full;
        }
        // Array and spill cells need the full pass's two-phase ordering. Dynamic
        // refs are a separate set; they do not force full by themselves.
        if affected
            .iter()
            .chain(dyn_cone.iter())
            .any(|cell| self.graph.arrays.contains(cell))
        {
            self.evaluate_full_untracked();
            return EvalPass::Full;
        }
        // Recompute the affected cells and collect the ones whose value actually
        // moved. A cycle in the affected set has no topological order, so fall
        // back to recomputing the whole set, where `evaluate_cell`'s recursion
        // still reports `#CIRC!`.
        let mut changed = match self.graph.topo_order(&affected) {
            Some(order) => self.recompute_frontier(affected, &seeds, &seeds, order),
            None => self.recompute_all(affected, &seeds),
        };
        // OFFSET/INDIRECT run after the static frontier so they read updated
        // targets, then their dependents pick up the new value. They have no
        // static edges through a helper, so drop the eval memo on the whole
        // cone first: otherwise A1=OFFSET(...) can read D2=E2 while D2 is still
        // Evaluated. Keep links: a HYPERLINK dependent that the frontier then
        // skips would otherwise lose its URL. They must run, but they are not
        // user edits: only report them when observable state moved.
        if !dyn_seeds.is_empty() {
            for &position in &dyn_cone {
                self.cells.remove(&position);
            }
            let dyn_changed = match self.graph.topo_order(&dyn_cone) {
                Some(order) => self.recompute_frontier(dyn_cone, &dyn_seeds, &[], order),
                None => self.recompute_all(dyn_cone, &[]),
            };
            changed.extend(dyn_changed);
        }
        // Record only the changed cells for `take_changed_cells`, unless a full
        // pass has already marked everything changed since the last read, or an
        // insert/delete moved cells the dirty cone does not name.
        if self.graph.take_structural_unknown() {
            self.changed_cells = ChangedCells::All;
        } else if let ChangedCells::Delta(delta) = &mut self.changed_cells {
            delta.extend(changed);
        }
        self.graph.after_pass();
        let cf_before = self.cf_cache.clone();
        self.evaluate_conditional_formatting();
        self.record_cf_changes(cf_before);
        EvalPass::Incremental
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

    /// Recomputes `must_run` in topological order. `always_report` (user edits,
    /// RAND) always counts as changed and propagates. Phase 2 OFFSET/INDIRECT
    /// must run but only report when observable state moved. An unchanged
    /// non-report cell stops the fanout there.
    fn recompute_frontier(
        &mut self,
        affected: HashSet<Position>,
        must_run: &[Position],
        always_report: &[Position],
        order: Vec<Position>,
    ) -> Vec<Position> {
        let before: HashMap<Position, Option<ChangeKey>> =
            affected.iter().map(|&p| (p, self.change_key(p))).collect();
        self.recompute_scope = Some(affected.clone());
        let report: HashSet<Position> = always_report.iter().copied().collect();
        let mut stale: HashSet<Position> = must_run.iter().copied().collect();
        let mut changed = HashSet::new();
        for position in order {
            if !stale.contains(&position) {
                continue;
            }
            self.invalidate(position);
            let (sheet, row, column) = position;
            self.evaluate_cell(CellReferenceIndex { sheet, row, column });
            if report.contains(&position) || self.change_key(position) != before[&position] {
                changed.insert(position);
                stale.extend(self.graph.dependents_of(position));
            }
        }
        self.recompute_scope = None;
        // OFFSET/INDIRECT can recompute a helper via evaluate_cell before this
        // loop reaches it; that cell never entered `stale`.
        for &position in &affected {
            if report.contains(&position) || self.change_key(position) != before[&position] {
                changed.insert(position);
            }
        }
        changed.into_iter().collect()
    }

    /// Recomputes the whole affected set, used when a cycle prevents ordering.
    /// Returns `always_report` plus every other cell whose value moved.
    fn recompute_all(
        &mut self,
        affected: HashSet<Position>,
        always_report: &[Position],
    ) -> Vec<Position> {
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
        let report: HashSet<Position> = always_report.iter().copied().collect();
        order
            .into_iter()
            .filter(|p| report.contains(p) || self.change_key(*p) != before[p])
            .collect()
    }

    /// Full recompute whose result is not expressible as a delta: it may have
    /// changed any cell, so the next `take_changed_cells` reports `Everything`.
    pub(crate) fn evaluate_full_untracked(&mut self) {
        self.evaluate_full();
        self.changed_cells = ChangedCells::All;
    }

    /// Returns the cells whose observable state moved on incremental evaluations
    /// since the last call, sorted, and clears the record. `Everything` means a
    /// full recompute has run, or an insert/delete moved cells the dirty cone
    /// cannot name. An empty `Cells` delta is not `Everything`.
    pub fn take_changed_cells(&mut self) -> ChangedSinceRead {
        // Reading re-arms tracking: the record resets to an empty delta, so
        // subsequent incremental passes accumulate afresh.
        let taken = std::mem::replace(&mut self.changed_cells, ChangedCells::Delta(HashSet::new()));
        let ChangedCells::Delta(cells) = taken else {
            return ChangedSinceRead::Everything;
        };
        let mut cells: Vec<Position> = cells.into_iter().collect();
        cells.sort_unstable();
        ChangedSinceRead::Cells(
            cells
                .into_iter()
                .map(|(sheet, row, column)| CellReferenceIndex { sheet, row, column })
                .collect(),
        )
    }

    /// Performance-only: a wide cone is cheaper as a full pass. Verify stays on
    /// the incremental path so the oracle still compares the two.
    fn should_fallback_fanout(&self, fanout: usize) -> bool {
        #[cfg(feature = "recalc_verify")]
        if self.recalc_mode == RecalcMode::Verify {
            return false;
        }
        self.formula_cell_count >= INCREMENTAL_FANOUT_FLOOR
            && fanout * INCREMENTAL_FANOUT_RATIO >= self.formula_cell_count
    }
}

/// Unions two change records. `All` wins; otherwise the cells are merged.
#[cfg(feature = "recalc_verify")]
fn merge_changed_cells(consumer: ChangedCells, this_pass: ChangedCells) -> ChangedCells {
    match (consumer, this_pass) {
        (ChangedCells::All, _) | (_, ChangedCells::All) => ChangedCells::All,
        (ChangedCells::Delta(mut a), ChangedCells::Delta(b)) => {
            a.extend(b);
            ChangedCells::Delta(a)
        }
    }
}
