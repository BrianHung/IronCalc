//! Shared differential-fuzz harness: runs an identical operation sequence on a
//! `RecalcMode::Full` model and a `RecalcMode::Incremental` model (and, with the
//! `recalc_verify` feature, a `RecalcMode::Verify` model), comparing every
//! populated cell after every `evaluate()`, and checking the incremental
//! changed-cells delta for completeness and soundness.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ironcalc_base::expressions::types::Area;
use ironcalc_base::types::{CellType, Color, Style};
use ironcalc_base::{Model, RecalcMode};
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{catch_unwind, AssertUnwindSafe};

// ---------------------------------------------------------------------------
// RNG
// ---------------------------------------------------------------------------

pub struct Lcg(pub u64);
impl Lcg {
    pub fn new(seed: u64) -> Self {
        Lcg(seed.wrapping_mul(0x9E3779B97F4A7C15) ^ 0xD1B54A32D192ED03)
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo + 1) as u64) as i32
    }
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

pub type AreaTuple = (u32, i32, i32, i32, i32); // sheet, row, col, width, height

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// `Model::set_user_input`
    Set {
        sheet: u32,
        row: i32,
        col: i32,
        value: String,
    },
    /// `Model::update_cell_with_number`
    SetNumber {
        sheet: u32,
        row: i32,
        col: i32,
        value: f64,
    },
    /// `Model::update_cell_with_text`
    SetText {
        sheet: u32,
        row: i32,
        col: i32,
        value: String,
    },
    /// `Model::update_cell_with_bool`
    SetBool {
        sheet: u32,
        row: i32,
        col: i32,
        value: bool,
    },
    /// `Model::set_user_array_formula` (CSE array)
    ArrayFormula {
        sheet: u32,
        row: i32,
        col: i32,
        width: i32,
        height: i32,
        formula: String,
    },
    InsertRows {
        sheet: u32,
        row: i32,
        count: i32,
    },
    DeleteRows {
        sheet: u32,
        row: i32,
        count: i32,
    },
    InsertCols {
        sheet: u32,
        col: i32,
        count: i32,
    },
    DeleteCols {
        sheet: u32,
        col: i32,
        count: i32,
    },
    MoveRows {
        sheet: u32,
        row: i32,
        count: i32,
        delta: i32,
    },
    MoveCols {
        sheet: u32,
        col: i32,
        count: i32,
        delta: i32,
    },
    HideRow {
        sheet: u32,
        row: i32,
        hidden: bool,
    },
    HideCol {
        sheet: u32,
        col: i32,
        hidden: bool,
    },
    /// `Model::set_cell_style`; variant selects fill colour / number format.
    CellStyle {
        sheet: u32,
        row: i32,
        col: i32,
        variant: u8,
    },
    NewName {
        name: String,
        scope: Option<u32>,
        formula: String,
    },
    UpdateName {
        name: String,
        scope: Option<u32>,
        new_name: String,
        new_scope: Option<u32>,
        formula: String,
    },
    DeleteName {
        name: String,
        scope: Option<u32>,
    },
    AddSheet {
        name: String,
    },
    DeleteSheet {
        index: u32,
    },
    RenameSheet {
        index: u32,
        name: String,
    },
    ClearContents {
        area: AreaTuple,
    },
    ClearAll {
        area: AreaTuple,
    },
    Evaluate,
}

impl Op {
    pub fn kind(&self) -> &'static str {
        match self {
            Op::Set { .. } => "Set",
            Op::SetNumber { .. } => "SetNumber",
            Op::SetText { .. } => "SetText",
            Op::SetBool { .. } => "SetBool",
            Op::ArrayFormula { .. } => "ArrayFormula",
            Op::InsertRows { .. } => "InsertRows",
            Op::DeleteRows { .. } => "DeleteRows",
            Op::InsertCols { .. } => "InsertCols",
            Op::DeleteCols { .. } => "DeleteCols",
            Op::MoveRows { .. } => "MoveRows",
            Op::MoveCols { .. } => "MoveCols",
            Op::HideRow { .. } => "HideRow",
            Op::HideCol { .. } => "HideCol",
            Op::CellStyle { .. } => "CellStyle",
            Op::NewName { .. } => "NewName",
            Op::UpdateName { .. } => "UpdateName",
            Op::DeleteName { .. } => "DeleteName",
            Op::AddSheet { .. } => "AddSheet",
            Op::DeleteSheet { .. } => "DeleteSheet",
            Op::RenameSheet { .. } => "RenameSheet",
            Op::ClearContents { .. } => "ClearContents",
            Op::ClearAll { .. } => "ClearAll",
            Op::Evaluate => "Evaluate",
        }
    }

    /// Cells this op writes as a plain value (incremental "seeds").
    pub fn value_seed(&self) -> Option<(u32, i32, i32)> {
        match self {
            Op::Set {
                sheet,
                row,
                col,
                value,
            } if !value.is_empty() && !value.starts_with('\'') => Some((*sheet, *row, *col)),
            Op::SetNumber {
                sheet, row, col, ..
            }
            | Op::SetText {
                sheet, row, col, ..
            }
            | Op::SetBool {
                sheet, row, col, ..
            }
            | Op::ArrayFormula {
                sheet, row, col, ..
            } => Some((*sheet, *row, *col)),
            _ => None,
        }
    }

    /// Valid Rust source for this op, for pasting into a repro test.
    pub fn to_rust(&self) -> String {
        fn s(v: &str) -> String {
            format!("{v:?}.to_string()")
        }
        match self {
            Op::Set {
                sheet,
                row,
                col,
                value,
            } => format!(
                "Op::Set {{ sheet: {sheet}, row: {row}, col: {col}, value: {} }}",
                s(value)
            ),
            Op::SetNumber {
                sheet,
                row,
                col,
                value,
            } => format!("Op::SetNumber {{ sheet: {sheet}, row: {row}, col: {col}, value: {value:?} }}"),
            Op::SetText {
                sheet,
                row,
                col,
                value,
            } => format!(
                "Op::SetText {{ sheet: {sheet}, row: {row}, col: {col}, value: {} }}",
                s(value)
            ),
            Op::SetBool {
                sheet,
                row,
                col,
                value,
            } => format!("Op::SetBool {{ sheet: {sheet}, row: {row}, col: {col}, value: {value} }}"),
            Op::ArrayFormula {
                sheet,
                row,
                col,
                width,
                height,
                formula,
            } => format!(
                "Op::ArrayFormula {{ sheet: {sheet}, row: {row}, col: {col}, width: {width}, height: {height}, formula: {} }}",
                s(formula)
            ),
            Op::InsertRows { sheet, row, count } => {
                format!("Op::InsertRows {{ sheet: {sheet}, row: {row}, count: {count} }}")
            }
            Op::DeleteRows { sheet, row, count } => {
                format!("Op::DeleteRows {{ sheet: {sheet}, row: {row}, count: {count} }}")
            }
            Op::InsertCols { sheet, col, count } => {
                format!("Op::InsertCols {{ sheet: {sheet}, col: {col}, count: {count} }}")
            }
            Op::DeleteCols { sheet, col, count } => {
                format!("Op::DeleteCols {{ sheet: {sheet}, col: {col}, count: {count} }}")
            }
            Op::MoveRows {
                sheet,
                row,
                count,
                delta,
            } => format!("Op::MoveRows {{ sheet: {sheet}, row: {row}, count: {count}, delta: {delta} }}"),
            Op::MoveCols {
                sheet,
                col,
                count,
                delta,
            } => format!("Op::MoveCols {{ sheet: {sheet}, col: {col}, count: {count}, delta: {delta} }}"),
            Op::HideRow { sheet, row, hidden } => {
                format!("Op::HideRow {{ sheet: {sheet}, row: {row}, hidden: {hidden} }}")
            }
            Op::HideCol { sheet, col, hidden } => {
                format!("Op::HideCol {{ sheet: {sheet}, col: {col}, hidden: {hidden} }}")
            }
            Op::CellStyle {
                sheet,
                row,
                col,
                variant,
            } => format!("Op::CellStyle {{ sheet: {sheet}, row: {row}, col: {col}, variant: {variant} }}"),
            Op::NewName {
                name,
                scope,
                formula,
            } => format!(
                "Op::NewName {{ name: {}, scope: {scope:?}, formula: {} }}",
                s(name),
                s(formula)
            ),
            Op::UpdateName {
                name,
                scope,
                new_name,
                new_scope,
                formula,
            } => format!(
                "Op::UpdateName {{ name: {}, scope: {scope:?}, new_name: {}, new_scope: {new_scope:?}, formula: {} }}",
                s(name),
                s(new_name),
                s(formula)
            ),
            Op::DeleteName { name, scope } => {
                format!("Op::DeleteName {{ name: {}, scope: {scope:?} }}", s(name))
            }
            Op::AddSheet { name } => format!("Op::AddSheet {{ name: {} }}", s(name)),
            Op::DeleteSheet { index } => format!("Op::DeleteSheet {{ index: {index} }}"),
            Op::RenameSheet { index, name } => {
                format!("Op::RenameSheet {{ index: {index}, name: {} }}", s(name))
            }
            Op::ClearContents { area } => format!("Op::ClearContents {{ area: {area:?} }}"),
            Op::ClearAll { area } => format!("Op::ClearAll {{ area: {area:?} }}"),
            Op::Evaluate => "Op::Evaluate".to_string(),
        }
    }
}

pub fn ops_to_rust(ops: &[Op]) -> String {
    let mut out = String::from("vec![\n");
    for op in ops {
        out.push_str("    ");
        out.push_str(&op.to_rust());
        out.push_str(",\n");
    }
    out.push(']');
    out
}

fn area(t: AreaTuple) -> Area {
    Area {
        sheet: t.0,
        row: t.1,
        column: t.2,
        width: t.3,
        height: t.4,
    }
}

/// Applies one op to a model. `Evaluate` calls `evaluate()`.
pub fn apply(model: &mut Model<'static>, op: &Op) -> Result<(), String> {
    match op {
        Op::Set {
            sheet,
            row,
            col,
            value,
        } => model.set_user_input(*sheet, *row, *col, value.clone()),
        Op::SetNumber {
            sheet,
            row,
            col,
            value,
        } => model.update_cell_with_number(*sheet, *row, *col, *value),
        Op::SetText {
            sheet,
            row,
            col,
            value,
        } => model.update_cell_with_text(*sheet, *row, *col, value),
        Op::SetBool {
            sheet,
            row,
            col,
            value,
        } => model.update_cell_with_bool(*sheet, *row, *col, *value),
        Op::ArrayFormula {
            sheet,
            row,
            col,
            width,
            height,
            formula,
        } => model.set_user_array_formula(*sheet, *row, *col, *width, *height, formula),
        Op::InsertRows { sheet, row, count } => model.insert_rows(*sheet, *row, *count),
        Op::DeleteRows { sheet, row, count } => model.delete_rows(*sheet, *row, *count),
        Op::InsertCols { sheet, col, count } => model.insert_columns(*sheet, *col, *count),
        Op::DeleteCols { sheet, col, count } => model.delete_columns(*sheet, *col, *count),
        Op::MoveRows {
            sheet,
            row,
            count,
            delta,
        } => model.move_rows_action(*sheet, *row, *count, *delta),
        Op::MoveCols {
            sheet,
            col,
            count,
            delta,
        } => model.move_columns_action(*sheet, *col, *count, *delta),
        Op::HideRow { sheet, row, hidden } => model.set_row_hidden(*sheet, *row, *hidden),
        Op::HideCol { sheet, col, hidden } => model.set_column_hidden(*sheet, *col, *hidden),
        Op::CellStyle {
            sheet,
            row,
            col,
            variant,
        } => {
            let mut style: Style = model.get_style_for_cell(*sheet, *row, *col)?;
            match variant % 4 {
                0 => style.fill.color = Color::Rgb("#FFAA00".to_string()),
                1 => style.num_fmt = "0.00".to_string(),
                2 => style.font.b = !style.font.b,
                _ => style.num_fmt = "#,##0".to_string(),
            }
            model.set_cell_style(*sheet, *row, *col, &style)
        }
        Op::NewName {
            name,
            scope,
            formula,
        } => model.new_defined_name(name, *scope, formula),
        Op::UpdateName {
            name,
            scope,
            new_name,
            new_scope,
            formula,
        } => model.update_defined_name(name, *scope, new_name, *new_scope, formula),
        Op::DeleteName { name, scope } => model.delete_defined_name(name, *scope),
        Op::AddSheet { name } => model.add_sheet(name),
        Op::DeleteSheet { index } => model.delete_sheet(*index),
        Op::RenameSheet { index, name } => model.rename_sheet_by_index(*index, name),
        Op::ClearContents { area: a } => model.range_clear_contents(&area(*a)),
        Op::ClearAll { area: a } => model.range_clear_all(&area(*a)),
        Op::Evaluate => {
            model.evaluate();
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshots and comparison
// ---------------------------------------------------------------------------

pub type Pos = (u32, i32, i32);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CellKey {
    /// `Debug` of `CellValue`
    pub value: String,
    /// `Debug` of `CellType`
    pub ty: String,
    pub formatted: String,
    pub formula: Option<String>,
    /// `Debug` of the cell's link (static or dynamic HYPERLINK), if any. Part of
    /// the observable state the delta must report (a URL can move under a
    /// fixed label).
    pub link: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub sheets: Vec<String>,
    pub cells: BTreeMap<Pos, CellKey>,
    /// Per sheet `Debug` of `get_links_list` (dynamic HYPERLINK links included)
    pub links: Vec<String>,
}

pub fn snapshot(model: &Model<'_>) -> Snapshot {
    let sheets: Vec<String> = model
        .get_worksheets_properties()
        .into_iter()
        .map(|p| p.name)
        .collect();
    let mut cells = BTreeMap::new();
    for c in model.get_all_cells() {
        let pos = (c.index, c.row, c.column);
        let value = format!(
            "{:?}",
            model.get_cell_value_by_index(c.index, c.row, c.column)
        );
        let ty = format!("{:?}", model.get_cell_type(c.index, c.row, c.column));
        let formatted = model
            .get_formatted_cell_value(c.index, c.row, c.column)
            .unwrap_or_else(|e| format!("ERR<{e}>"));
        let formula = model
            .get_cell_formula(c.index, c.row, c.column)
            .ok()
            .flatten();
        cells.insert(
            pos,
            CellKey {
                value,
                ty,
                formatted,
                formula,
                link: String::new(),
            },
        );
    }
    let mut links = Vec::new();
    for s in 0..sheets.len() as u32 {
        let mut l = model.get_links_list(s).unwrap_or_default();
        l.sort_by_key(|v| (v.row, v.column));
        for view in &l {
            let pos = (s, view.row, view.column);
            // A linked position with no cell reads exactly like an EmptyCell
            // through the public API (value None, type Number), so synthesize
            // the same key; a style write that materializes the EmptyCell is
            // not an observable change and is not journaled.
            let entry = cells.entry(pos).or_insert_with(|| CellKey {
                value: "Ok(None)".to_string(),
                ty: format!("{:?}", Ok::<CellType, String>(CellType::Number)),
                formatted: String::new(),
                formula: None,
                link: String::new(),
            });
            entry.link = format!("{view:?}");
        }
        links.push(format!("{l:?}"));
    }
    Snapshot {
        sheets,
        cells,
        links,
    }
}

/// Numbers that differ only by floating-point association noise are not a
/// divergence per the documented `RecalcMode::Incremental` contract.
fn fp_noise(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Option<f64> {
        s.strip_prefix("Ok(Number(")
            .and_then(|s| s.strip_suffix("))"))
            .and_then(|s| s.parse::<f64>().ok())
    };
    match (parse(a), parse(b)) {
        (Some(x), Some(y)) => (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0),
        _ => false,
    }
}

#[derive(Clone, Debug)]
pub struct Failure {
    /// Index of the `Evaluate` op (in the scenario) at which the failure was seen.
    pub step: usize,
    pub kind: String,
    pub detail: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] at op #{}: {}", self.kind, self.step, self.detail)
    }
}

pub fn diff_snapshots(
    label_a: &str,
    a: &Snapshot,
    label_b: &str,
    b: &Snapshot,
) -> Option<(String, String)> {
    if a.sheets != b.sheets {
        return Some((
            "sheet-mismatch".into(),
            format!(
                "{label_a} sheets {:?} vs {label_b} sheets {:?}",
                a.sheets, b.sheets
            ),
        ));
    }
    let keys: BTreeSet<&Pos> = a.cells.keys().chain(b.cells.keys()).collect();
    let mut fp_only: Option<String> = None;
    for pos in keys {
        let ka = a.cells.get(pos);
        let kb = b.cells.get(pos);
        if ka == kb {
            continue;
        }
        let formula = ka
            .and_then(|k| k.formula.clone())
            .or_else(|| kb.and_then(|k| k.formula.clone()))
            .unwrap_or_default();
        match (ka, kb) {
            (Some(x), Some(y)) => {
                if x.value != y.value || x.ty != y.ty {
                    if fp_noise(&x.value, &y.value) && x.ty == y.ty {
                        fp_only.get_or_insert(format!(
                            "fp-noise at {pos:?} ({formula}): {label_a}={} {label_b}={}",
                            x.value, y.value
                        ));
                        continue;
                    }
                    return Some((
                        "value-divergence".into(),
                        format!(
                            "cell {pos:?} formula={formula:?}: {label_a}={} ({}) vs {label_b}={} ({})",
                            x.value, x.ty, y.value, y.ty
                        ),
                    ));
                }
                if x.formula != y.formula {
                    return Some((
                        "formula-divergence".into(),
                        format!(
                            "cell {pos:?}: {label_a} formula={:?} vs {label_b} formula={:?}",
                            x.formula, y.formula
                        ),
                    ));
                }
                if x.formatted != y.formatted {
                    if fp_noise(&x.value, &y.value) {
                        continue;
                    }
                    return Some((
                        "format-divergence".into(),
                        format!(
                            "cell {pos:?} formula={formula:?}: {label_a}={:?} vs {label_b}={:?}",
                            x.formatted, y.formatted
                        ),
                    ));
                }
            }
            (Some(x), None) => {
                return Some((
                    "value-divergence".into(),
                    format!(
                        "cell {pos:?} formula={formula:?}: {label_a}={} ({}) vs {label_b}=<absent>",
                        x.value, x.ty
                    ),
                ))
            }
            (None, Some(y)) => {
                return Some((
                    "value-divergence".into(),
                    format!(
                        "cell {pos:?} formula={formula:?}: {label_a}=<absent> vs {label_b}={} ({})",
                        y.value, y.ty
                    ),
                ))
            }
            (None, None) => unreachable!(),
        }
    }
    if a.links != b.links {
        return Some((
            "link-divergence".into(),
            format!(
                "{label_a} links {:?} vs {label_b} links {:?}",
                a.links, b.links
            ),
        ));
    }
    let _ = fp_only;
    None
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

#[derive(Default, Debug, Clone)]
pub struct Stats {
    pub evaluates: usize,
    pub cells_deltas: usize,
    pub everything_deltas: usize,
    pub ops_applied: usize,
    pub ops_rejected: usize,
}

pub struct Harness {
    pub full: Model<'static>,
    pub incr: Model<'static>,
    #[cfg(feature = "recalc_verify")]
    pub verify: Option<Model<'static>>,
    pub check_delta: bool,
    pub check_verify: bool,
    seeds: BTreeSet<Pos>,
    visibility_edit: bool,
    last: Snapshot,
    pub stats: Stats,
}

fn fresh(mode: RecalcMode) -> Model<'static> {
    Model::new_empty("fuzz", "en", "UTC", "en")
        .unwrap()
        .with_recalc_mode(mode)
}

impl Harness {
    pub fn new(check_delta: bool, check_verify: bool) -> Self {
        let incr = fresh(RecalcMode::Incremental);
        let last = snapshot(&incr);
        Harness {
            full: fresh(RecalcMode::Full),
            incr,
            #[cfg(feature = "recalc_verify")]
            verify: if check_verify {
                Some(fresh(RecalcMode::Verify))
            } else {
                None
            },
            check_delta,
            check_verify,
            seeds: BTreeSet::new(),
            visibility_edit: false,
            last,
            stats: Stats::default(),
        }
    }

    /// Applies `op` to every model and, on `Evaluate`, compares them.
    pub fn step(&mut self, index: usize, op: &Op) -> Result<(), Failure> {
        let r_full = apply(&mut self.full, op);
        let r_incr = apply(&mut self.incr, op);
        if r_full.is_ok() != r_incr.is_ok() {
            return Err(Failure {
                step: index,
                kind: "result-mismatch".into(),
                detail: format!("op {op:?}: full={r_full:?} incr={r_incr:?}"),
            });
        }
        #[cfg(feature = "recalc_verify")]
        if let Some(v) = self.verify.as_mut() {
            let r_verify = apply(v, op);
            if r_verify.is_ok() != r_full.is_ok() {
                return Err(Failure {
                    step: index,
                    kind: "result-mismatch".into(),
                    detail: format!("op {op:?}: full={r_full:?} verify={r_verify:?}"),
                });
            }
        }
        if r_full.is_err() {
            self.stats.ops_rejected += 1;
            return Ok(());
        }
        self.stats.ops_applied += 1;
        if let Some(seed) = op.value_seed() {
            self.seeds.insert(seed);
        }
        if matches!(op, Op::HideRow { .. } | Op::HideCol { .. }) {
            self.visibility_edit = true;
        }
        if !matches!(op, Op::Evaluate) {
            return Ok(());
        }
        self.stats.evaluates += 1;
        let full = snapshot(&self.full);
        let incr = snapshot(&self.incr);
        if let Some((kind, detail)) = diff_snapshots("Full", &full, "Incremental", &incr) {
            return Err(Failure {
                step: index,
                kind,
                detail,
            });
        }
        #[cfg(feature = "recalc_verify")]
        if let Some(v) = self.verify.as_ref() {
            let verify = snapshot(v);
            if let Some((kind, detail)) = diff_snapshots("Full", &full, "Verify", &verify) {
                return Err(Failure {
                    step: index,
                    kind: format!("verify-{kind}"),
                    detail,
                });
            }
        }
        // BEGIN-DELTA
        if self.check_delta {
            use ironcalc_base::ChangedSinceRead;
            match self.incr.take_changed_cells() {
                ChangedSinceRead::Everything => self.stats.everything_deltas += 1,
                ChangedSinceRead::Cells(cells) => {
                    self.stats.cells_deltas += 1;
                    let delta: BTreeSet<Pos> =
                        cells.iter().map(|c| (c.sheet, c.row, c.column)).collect();
                    // A style-only write creates an empty cell; that is not a value change.
                    let observable = |k: Option<&CellKey>| {
                        k.filter(|k| k.value != "Ok(None)" || !k.link.is_empty())
                            .map(|k| (k.value.clone(), k.ty.clone(), k.link.clone()))
                    };
                    let keys: BTreeSet<&Pos> =
                        self.last.cells.keys().chain(incr.cells.keys()).collect();
                    for pos in keys {
                        let before = observable(self.last.cells.get(pos));
                        let after = observable(incr.cells.get(pos));
                        let changed = before != after;
                        if changed && !delta.contains(pos) {
                            let formula = incr
                                .cells
                                .get(pos)
                                .and_then(|k| k.formula.clone())
                                .unwrap_or_default();
                            return Err(Failure {
                                step: index,
                                kind: "delta-missing".into(),
                                detail: format!(
                                    "cell {pos:?} formula={formula:?} changed {before:?} -> {after:?} but delta={delta:?}"
                                ),
                            });
                        }
                    }
                    for pos in &delta {
                        let before = observable(self.last.cells.get(pos));
                        let after = observable(incr.cells.get(pos));
                        if before != after || self.seeds.contains(pos) {
                            continue;
                        }
                        let formula = incr
                            .cells
                            .get(pos)
                            .and_then(|k| k.formula.clone())
                            .unwrap_or_default();
                        if self.visibility_edit && formula.to_uppercase().contains("SUBTOTAL") {
                            continue;
                        }
                        return Err(Failure {
                            step: index,
                            kind: "delta-unsound".into(),
                            detail: format!(
                                "cell {pos:?} formula={formula:?} is in the delta but did not change (value {after:?}); seeds={:?}",
                                self.seeds
                            ),
                        });
                    }
                }
            }
        }
        // END-DELTA
        self.seeds.clear();
        self.visibility_edit = false;
        self.last = incr;
        Ok(())
    }
}

thread_local! {
    static QUIET: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Installs a panic hook that stays silent while `QUIET` is set (minimization).
pub fn install_quiet_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !QUIET.with(|q| q.get()) {
            default(info);
        }
    }));
}

pub fn set_quiet(q: bool) {
    QUIET.with(|c| c.set(q));
}

pub fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic>".to_string()
    }
}

/// Runs a scenario on fresh models; the first failure (or panic) is returned.
pub fn run_scenario(ops: &[Op], check_delta: bool, check_verify: bool) -> Result<Stats, Failure> {
    let mut h = Harness::new(check_delta, check_verify);
    let mut current = 0usize;
    let result = catch_unwind(AssertUnwindSafe(|| {
        for (i, op) in ops.iter().enumerate() {
            current = i;
            h.step(i, op)?;
        }
        Ok(h.stats.clone())
    }));
    match result {
        Ok(r) => r,
        Err(payload) => {
            let msg = panic_message(payload);
            let kind = if msg.contains("diverged") {
                "verify-panic-diverged"
            } else if msg.contains("missing from the delta") {
                "verify-panic-delta-missing"
            } else if msg.contains("did not change") {
                "verify-panic-delta-unsound"
            } else {
                // Distinguish panics by their message so distinct crashes do not
                // collapse into one bucket during minimization.
                let first = msg.lines().next().unwrap_or("");
                let head: String = first.chars().take(40).collect();
                return Err(Failure {
                    step: current,
                    kind: format!("panic:{head}"),
                    detail: format!("op {:?}: {}", ops.get(current), msg.replace('\n', " | ")),
                });
            };
            Err(Failure {
                step: current,
                kind: kind.into(),
                detail: format!("op {:?}: {}", ops.get(current), msg.replace('\n', " | ")),
            })
        }
    }
}

/// Reruns a scenario `tries` times and returns how many runs were clean. A
/// non-zero count on a failing scenario means the engine is nondeterministic.
pub fn clean_runs(ops: &[Op], check_delta: bool, check_verify: bool, tries: usize) -> usize {
    set_quiet(true);
    let n = (0..tries)
        .filter(|_| run_scenario(ops, check_delta, check_verify).is_ok())
        .count();
    set_quiet(false);
    n
}

/// Shrinks `ops` to a (locally) minimal sequence that still fails with the same
/// failure kind. Delta-debugging by chunk removal, then single-op removal.
pub fn minimize(
    ops: &[Op],
    kind: &str,
    check_delta: bool,
    check_verify: bool,
) -> (Vec<Op>, Failure) {
    minimize_by(ops, kind, |cand| {
        run_scenario(cand, check_delta, check_verify).map(|_| ())
    })
}

/// Generic minimizer: `run` replays a candidate and returns its first failure.
pub fn minimize_by<O: Clone>(
    ops: &[O],
    kind: &str,
    run: impl Fn(&[O]) -> Result<(), Failure>,
) -> (Vec<O>, Failure) {
    set_quiet(true);
    let fails = |cand: &[O]| -> Option<Failure> {
        match run(cand) {
            Err(f) if f.kind == kind => Some(f),
            _ => None,
        }
    };
    let mut cur: Vec<O> = ops.to_vec();
    let mut last_failure = run(&cur).expect_err("minimize: scenario must fail");
    // Truncate after the failing step.
    if last_failure.step + 1 < cur.len() {
        let cand: Vec<O> = cur[..=last_failure.step].to_vec();
        if let Some(f) = fails(&cand) {
            cur = cand;
            last_failure = f;
        }
    }
    let mut chunk = (cur.len() / 2).max(1);
    loop {
        let mut progressed = false;
        let mut i = 0;
        while i < cur.len() {
            let end = (i + chunk).min(cur.len());
            let mut cand = cur.clone();
            cand.drain(i..end);
            if !cand.is_empty() {
                if let Some(f) = fails(&cand) {
                    cur = cand;
                    last_failure = f;
                    progressed = true;
                    continue;
                }
            }
            i = end;
        }
        if chunk == 1 {
            if !progressed {
                break;
            }
        } else {
            chunk = (chunk / 2).max(1);
        }
    }
    set_quiet(false);
    (cur, last_failure)
}

// ---------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------

pub struct GenConfig {
    pub steps: usize,
    /// Formula templates containing any of these substrings are not generated.
    pub avoid_formulas: Vec<&'static str>,
    /// Op kinds (see `Op::kind`) that are not generated.
    pub avoid_ops: Vec<&'static str>,
    /// Maximum rows the sheet may reach before inserts are skipped.
    pub max_rows: i32,
}

impl Default for GenConfig {
    fn default() -> Self {
        GenConfig {
            steps: 150,
            avoid_formulas: vec![],
            avoid_ops: vec![],
            max_rows: 40,
        }
    }
}

pub const DATA_ROWS: i32 = 12;
pub const DATA_COLS: i32 = 4; // A..D on Sheet1
pub const FORMULA_COL_FIRST: i32 = 5; // E
pub const FORMULA_COL_LAST: i32 = 8; // H
pub const FORMULA_ROWS: i32 = 20;

/// Formula templates. Placeholders: `{D}` data-sheet name, `{R}` row 1..12,
/// `{S}` row 1..6, `{C}` column A..D, `{F}` a formula-region ref (E..H x 1..20),
/// `{N}` digit 0..9, `{M}` 1..5.
pub const ZOO: &[&str] = &[
    // aggregates, single and multi column, overlapping prefixes / running totals
    "=SUM(A1:A5)",
    "=SUM(A1:A6)",
    "=SUM(A2:A6)",
    "=SUM($A$1:A{R})",
    "=SUM(A{S}:A{R})",
    "=SUM(A1:C6)",
    "=SUM(A1:D12)",
    "=SUM(B3:C9)",
    "=MIN(A1:D12)",
    "=MAX(A3:C9)",
    "=COUNT(A1:D12)",
    "=COUNTA(A1:D12)",
    "=COUNTBLANK(A1:D12)",
    "=AVERAGE(A1:B10)",
    "=AVERAGE({C}1:{C}6)",
    "=SUM({C}{S}:{C}{R})",
    "=SUM(A1:A10)+SUM(A11:A12)",
    "=SUM(A1:A5)-SUM(A1:A4)",
    "=PRODUCT(B1:B3)",
    "=SUMPRODUCT(A1:A5,B1:B5)",
    "=SUM(E1:E5)",
    "=SUM(E1:H3)",
    "=SUM({F},{F})",
    "=SUM(E:E)",
    "=SUM(3:3)",
    // conditional aggregates with criteria ranges smaller/larger/cross-sheet
    "=SUMIF(A1:A10,\">{N}\",B1:B10)",
    "=SUMIF(A1:A3,\">{N}\",B1:B10)",
    "=SUMIF(A1:A10,\">{N}\",B1:B3)",
    "=SUMIF(A1:A6,\"<{N}\")",
    "=SUMIF({D}!A1:A6,\">{N}\",A1:A6)",
    "=SUMIF(A1:A6,\">{N}\",{D}!B1:B6)",
    "=SUMIFS(B1:B10,A1:A10,\">{N}\")",
    "=SUMIFS(B1:B10,A1:A3,\">{N}\")",
    "=SUMIFS(B1:B3,A1:A10,\">{N}\")",
    "=SUMIFS(B1:B10,A1:A10,\">{N}\",C1:C10,\"<{N}0\")",
    "=SUMIFS({D}!B1:B6,{D}!A1:A6,\">{N}\")",
    "=SUMIFS(C1:C8,{D}!A1:A8,\">{N}\")",
    "=COUNTIF(A1:A12,\">{N}0\")",
    "=COUNTIF(A1:A12,{C}{R})",
    "=COUNTIF({D}!A1:A12,A{S})",
    "=COUNTIFS(A1:A5,\">{N}\",B1:B5,\"<{N}0\")",
    "=COUNTIFS(A1:A5,\">{N}\",B1:B12,\"<{N}0\")",
    "=COUNTIFS(A1:A12,\">{N}\",{D}!A1:A12,\">{N}\")",
    "=AVERAGEIF(A1:A6,\">{N}\",C1:C2)",
    "=AVERAGEIF(A1:A6,\">{N}\",C1:C12)",
    "=AVERAGEIFS(B1:B6,A1:A6,\">{N}\")",
    "=MAXIFS(B1:B8,A1:A8,\">{N}\")",
    "=MINIFS(B1:B8,A1:A8,\"<{N}0\")",
    // IF branches
    "=IF(A{S}>{N}0,SUM(B1:B12),SUM(C1:C12))",
    "=IF(A1>B1,A1,B1)",
    "=IF(A{S}>5,IF(B{S}>5,\"both\",\"a\"),IF(B{S}>5,\"b\",\"none\"))",
    "=IF({C}{R}>{N},{F},{D}!A{S})",
    "=IF(A1>B1,TRUE,\"no\")",
    "=IFERROR(1/{C}{R},\"div\")",
    "=IFERROR({F},-1)",
    "=CHOOSE(MOD(A{S},3)+1,A1,B1,C1)",
    // OFFSET value and range
    "=OFFSET(A1,MOD(B{S},5),MOD(C{S},3))",
    "=OFFSET(A1,1,1)",
    "=SUM(OFFSET(A1,0,0,MOD(B{S},8)+1,1))",
    "=SUM(OFFSET({D}!A1,0,0,3,MOD(A{S},2)+1))",
    "=OFFSET(E1,MOD(A{S},4),0)",
    "=IF(D1>D2,OFFSET(B1,2,0),C3)",
    // INDIRECT
    "=INDIRECT(\"A\"&(MOD(B{S},10)+1))",
    "=SUM(INDIRECT(\"{D}!A1:A\"&(MOD(A{S},6)+1)))",
    "=INDIRECT(\"{C}\"&(MOD({F},12)+1))",
    "=INDIRECT({F})",
    "=SUM(INDIRECT(\"A1:\"&\"C\"&(MOD(A{S},5)+1)))",
    // lookups
    "=INDEX(A1:C8,MOD(B{S},8)+1,MOD(C{S},3)+1)",
    "=INDEX(A1:A12,{R})",
    "=MATCH(A{S},A1:A10,0)",
    "=MATCH({N}0,A1:A12,1)",
    "=VLOOKUP(A{S},A1:C10,2,FALSE)",
    "=VLOOKUP({N}0,A1:D12,3,TRUE)",
    "=HLOOKUP(A1,A1:D3,2,FALSE)",
    "=XLOOKUP(B{S},B1:B10,C1:C10,\"nf\")",
    "=XLOOKUP(A{S},{D}!A1:A12,{D}!B1:B12,0)",
    "=INDEX({D}!A1:C6,MATCH(A{S},{D}!A1:A6,0),2)",
    "=XMATCH(A{S},A1:A12)",
    "=SMALL(A1:A12,{M})",
    "=LARGE(B1:B12,{M})",
    // structure functions
    "=ROW()",
    "=COLUMN()",
    "=ROW()+COLUMN()",
    "=ROW(A{R})",
    "=ROWS(A1:A8)",
    "=COLUMNS(A1:D1)",
    "=FORMULATEXT({F})",
    "=FORMULATEXT(E1)",
    "=ISFORMULA({F})",
    "=ISBLANK({C}{R})",
    "=ADDRESS(ROW(),COLUMN())",
    "=SHEET()",
    "=SHEETS()",
    // SUBTOTAL
    "=SUBTOTAL(9,A1:A10)",
    "=SUBTOTAL(109,A1:A10)",
    "=SUBTOTAL(102,B1:B10)",
    "=SUBTOTAL(1,A1:A5)",
    "=SUBTOTAL(101,{C}1:{C}8)",
    "=SUBTOTAL(9,E1:E6)",
    "=SUBTOTAL(103,A1:D12)",
    // HYPERLINK
    "=HYPERLINK(\"https://x.com/\"&A{S},\"L\"&B{S})",
    "=HYPERLINK({F},\"lbl\")",
    "=HYPERLINK(\"https://x.com/\"&{F})",
    // cross sheet
    "={D}!A1+{D}!B2",
    "=SUM({D}!A1:B6)",
    "=SUM({D}!A1:C12)",
    "={D}!D1",
    "={D}!E{S}*2",
    "={D}!{C}{R}",
    "=MAX({D}!A1:A12)+MIN(A1:A12)",
    "=SUM({D}!A:A)",
    // defined names
    "=NCELL*2",
    "=SUM(NRANGE)",
    "=SUM(NDATA)",
    "=SCELL+1",
    "=SUM(SRANGE)",
    "=SUMIF(NRANGE,\">{N}\",SRANGE)",
    "=COUNTIF(NRANGE,\">{N}\")",
    "=SUMIFS(SRANGE,NRANGE,\">{N}\")",
    "=COUNTIFS(NRANGE,\">{N}\",SRANGE,\"<{N}0\")",
    "=LAM({C}{R})",
    "=LAM(NCELL)",
    "=LET(x,NCELL,y,{C}{R},x*y)",
    "=LET(a,SUM(A1:A3),b,a*2,b+{F})",
    "=INDEX(NRANGE,{S})",
    "=MAX(NDATA)",
    "=NRANGE",
    "=LAMBDA(x,x+A{S})({N})",
    "=MAP(A1:A3,LAMBDA(v,v*2))",
    "=SUM(MAP(A1:A4,LAMBDA(v,v+B1)))",
    // dynamic arrays
    "=SEQUENCE(3)",
    "=SEQUENCE(MOD(A{S},3)+1)",
    "=SEQUENCE(2,2)",
    "=A1:A3",
    "=A{S}:A{S}",
    "={D}!A1:B2",
    "=FILTER(A1:A6,B1:B6>{N}0)",
    "=FILTER(A1:A6,B1:B6>{N}0,\"none\")",
    "=SORT(A1:A5)",
    "=SORT(A1:B4,2,-1)",
    "=UNIQUE(C1:C8)",
    "=TRANSPOSE(A1:C1)",
    "=A1:A3*2",
    "=A1:A3+B1:B3",
    "=SUM(A1:A3*B1:B3)",
    "=E15#",
    "=SUM(E15#)",
    "=COUNT(F15#)",
    "=SUMPRODUCT((A1:A8>{N})*B1:B8)",
    "=SEQUENCE(3)+{F}",
    "=INDEX(SORT(A1:A6),1)",
    "=SORTBY(A1:A4,B1:B4)",
    "=UNIQUE({D}!A1:A8)",
    // text
    "=A{S}&\"-\"&B{S}&TEXT(C{S},\"0\")",
    "=LEN({F}&{F})",
    "=CONCAT(A1:A3)",
    "={F}&\"x\"",
    "=TEXTJOIN(\",\",TRUE,A1:C2)",
    "=REPT(\"a\",MOD(A{S},4))",
    "=MID({F}&\"abcdef\",2,3)",
    "=TEXT(A{S},\"0.00\")&B{S}",
    "=UPPER({F})",
    // errors / booleans / misc arithmetic chains
    "=1/0",
    "=NA()",
    "=1/{C}{R}",
    "=A{S}>B{S}",
    "=AND(A{S}>5,B{S}<50)",
    "=NOT({F})",
    "=ISERROR({F})",
    "={F}+{F}",
    "={F}*2-{F}",
    "={F}-1",
    "=-{F}",
    "={C}{R}",
    "={C}{R}+{C}{R}",
    "={C}{R}^2",
    "=MOD(A{S},{M})",
    "=ROUND(A{S}/7,2)",
    "=ABS(A{S}-B{S})",
    "=INT({F}/3)",
    "=TRUE",
    "=\"text\"",
    "=42",
    // circular pair (placed as fixed cells too)
    "=G19+1",
    "=E20",
    "=SUM(E19:E20)",
];

/// Templates for the Data sheet (index 1). `{D}` is still the data sheet.
pub const DATA_ZOO: &[&str] = &[
    "=SUM(Sheet1!A1:A5)",
    "=Sheet1!E1",
    "=Sheet1!{C}{R}*2",
    "=SUM(A1:A6)",
    "=SUM(A{S}:B{R})",
    "=COUNTIF(Sheet1!A1:A12,\">{N}\")",
    "=SUMIF(A1:A6,\">{N}\",Sheet1!B1:B6)",
    "=D1+1",
    "=D{S}+E{S}",
    "=OFFSET(A1,MOD(B1,4),0)",
    "=INDIRECT(\"Sheet1!A\"&(MOD(A1,5)+1))",
    "=SUM(NRANGE)",
    "=NCELL",
    "=SEQUENCE(2)",
    "=A1:A2",
    "=HYPERLINK(\"https://d.com/\"&A1,B1)",
    "=Sheet1!E15#",
    "=SUBTOTAL(109,A1:A12)",
    "=ROW()",
    "=FORMULATEXT(D1)",
    "=MAX(A1:C12)",
    "=1/A{S}",
];

pub const NAME_POOL: &[(&str, Option<u32>)] = &[
    ("NCELL", None),
    ("NRANGE", None),
    ("NDATA", None),
    ("SCELL", Some(0)),
    ("SRANGE", Some(0)),
    ("LAM", None),
    ("NEW1", None),
    ("NEW2", Some(1)),
];

pub const NAME_TARGETS: &[&str] = &[
    "Sheet1!$A$1",
    "Sheet1!$A$2",
    "Sheet1!$B$2",
    "Sheet1!$E$1",
    "Sheet1!$A$1:$A$8",
    "Sheet1!$A$2:$A$9",
    "Sheet1!$B$1:$B$6",
    "Sheet1!$A$1:$D$12",
    "Sheet1!$E$1:$E$5",
    "{D}!$A$1:$B$6",
    "{D}!$B$3",
    "{D}!$A$1:$A$12",
    "LAMBDA(x,x*2+Sheet1!$A$2)",
    "LAMBDA(x,SUM(Sheet1!$A$1:$A$4)+x)",
    "LAMBDA(x,x+NCELL)",
    "Sheet1!$A$1:$A$3*2",
    "5",
];

pub fn col_letter(c: i32) -> String {
    // 1 -> A
    let mut s = String::new();
    let mut n = c;
    while n > 0 {
        let r = (n - 1) % 26;
        s.insert(0, (b'A' + r as u8) as char);
        n = (n - 1) / 26;
    }
    s
}

pub struct Generator {
    pub rng: Lcg,
    pub cfg: GenConfig,
    /// A shadow model used only to query structure (formula cells, sheets).
    pub shadow: Model<'static>,
    pub ops: Vec<Op>,
    /// Set once the shadow model panicked; generation stops there.
    pub panicked: bool,
}

impl Generator {
    pub fn new(seed: u64, cfg: GenConfig) -> Self {
        Generator {
            rng: Lcg::new(seed),
            cfg,
            shadow: fresh(RecalcMode::Full),
            ops: Vec::new(),
            panicked: false,
        }
    }

    pub fn sheet_names(&self) -> Vec<String> {
        self.shadow
            .get_worksheets_properties()
            .into_iter()
            .map(|p| p.name)
            .collect()
    }

    pub fn data_sheet_name(&self) -> String {
        self.sheet_names()
            .get(1)
            .cloned()
            .unwrap_or_else(|| "Data".to_string())
    }

    pub fn fill(&mut self, template: &str) -> String {
        let d = self.data_sheet_name();
        let mut out = String::new();
        let mut rest = template;
        while let Some(i) = rest.find('{') {
            out.push_str(&rest[..i]);
            let close = rest[i..].find('}').map(|j| i + j).unwrap_or(rest.len() - 1);
            let key = &rest[i + 1..close];
            let rep = match key {
                "D" => d.clone(),
                "R" => self.rng.range(1, DATA_ROWS).to_string(),
                "S" => self.rng.range(1, 6).to_string(),
                "C" => col_letter(self.rng.range(1, DATA_COLS)),
                "F" => format!(
                    "{}{}",
                    col_letter(self.rng.range(FORMULA_COL_FIRST, FORMULA_COL_LAST)),
                    self.rng.range(1, FORMULA_ROWS)
                ),
                "N" => self.rng.range(0, 9).to_string(),
                "M" => self.rng.range(1, 5).to_string(),
                other => format!("{{{other}}}"),
            };
            out.push_str(&rep);
            rest = &rest[close + 1..];
        }
        out.push_str(rest);
        out
    }

    fn template_allowed(&self, t: &str) -> bool {
        !self.cfg.avoid_formulas.iter().any(|a| t.contains(a))
    }

    fn op_allowed(&self, kind: &str) -> bool {
        !self.cfg.avoid_ops.contains(&kind)
    }

    pub fn random_formula(&mut self, sheet: u32) -> String {
        let zoo: Vec<&&str> = if sheet == 1 { DATA_ZOO } else { ZOO }
            .iter()
            .filter(|t| self.template_allowed(t))
            .collect();
        let t = **self.rng.pick(&zoo);
        self.fill(t)
    }

    fn push(&mut self, op: Op) {
        if !self.op_allowed(op.kind()) || self.panicked {
            return;
        }
        set_quiet(true);
        let r = catch_unwind(AssertUnwindSafe(|| apply(&mut self.shadow, &op)));
        set_quiet(false);
        if r.is_err() {
            // The engine panicked applying this op to a plain Full model. Stop
            // generating; the scenario ends here so the driver reports it.
            self.panicked = true;
        }
        self.ops.push(op);
    }

    fn random_value(&mut self) -> String {
        match self.rng.below(20) {
            0 => String::new(),
            1 => "TRUE".into(),
            2 => "FALSE".into(),
            3 => self
                .rng
                .pick(&["x", "abc", "hello", "10", "a1"])
                .to_string(),
            4 => "'7".into(),
            5 => "-5".into(),
            6 => "1000".into(),
            7 => "0".into(),
            8 => "#N/A".into(),
            9 => "https://ironcalc.com".into(),
            _ => self.rng.range(0, 99).to_string(),
        }
    }

    /// All formula cells of the shadow model.
    pub fn formula_cells(&self) -> Vec<Pos> {
        self.shadow
            .get_all_cells()
            .into_iter()
            .filter(|c| {
                self.shadow
                    .get_cell_formula(c.index, c.row, c.column)
                    .ok()
                    .flatten()
                    .is_some()
            })
            .map(|c| (c.index, c.row, c.column))
            .collect()
    }

    fn max_row(&self, sheet: u32) -> i32 {
        self.shadow
            .workbook
            .worksheet(sheet)
            .map(|w| w.dimension().max_row)
            .unwrap_or(1)
    }

    fn random_sheet(&mut self) -> u32 {
        let n = self.sheet_names().len() as u32;
        if self.rng.chance(70) {
            0
        } else {
            self.rng.below(n as u64) as u32
        }
    }

    /// Initial workbook: data on both sheets, names, formula zoo, fixed shapes.
    pub fn setup(&mut self) {
        self.push(Op::AddSheet {
            name: "Data".into(),
        });
        for row in 1..=DATA_ROWS {
            for col in 1..=DATA_COLS {
                let v = if self.rng.chance(8) {
                    self.random_value()
                } else {
                    self.rng.range(0, 60).to_string()
                };
                self.push(Op::Set {
                    sheet: 0,
                    row,
                    col,
                    value: v,
                });
            }
        }
        for row in 1..=DATA_ROWS {
            for col in 1..=3 {
                let v = self.rng.range(0, 40).to_string();
                self.push(Op::Set {
                    sheet: 1,
                    row,
                    col,
                    value: v,
                });
            }
        }
        let names: Vec<(String, Option<u32>, String)> = vec![
            ("NCELL".into(), None, "Sheet1!$A$1".into()),
            ("NRANGE".into(), None, "Sheet1!$A$1:$A$8".into()),
            ("NDATA".into(), None, "Data!$A$1:$B$6".into()),
            ("SCELL".into(), Some(0), "Sheet1!$B$2".into()),
            ("SRANGE".into(), Some(0), "Sheet1!$B$1:$B$6".into()),
            ("LAM".into(), None, "LAMBDA(x,x*2+Sheet1!$A$2)".into()),
        ];
        for (name, scope, formula) in names {
            self.push(Op::NewName {
                name,
                scope,
                formula,
            });
        }
        // Fixed shapes.
        let fixed: Vec<(u32, i32, i32, &str)> = vec![
            (0, 1, 8, "=SUM($A$1:A1)"),
            (0, 2, 8, "=SUM($A$1:A2)"),
            (0, 3, 8, "=SUM($A$1:A3)"),
            (0, 4, 8, "=SUM($A$1:A4)"),
            (0, 5, 8, "=SUM($A$1:A5)"),
            (0, 6, 8, "=SUM($A$1:A6)"),
            (0, 15, 5, "=SEQUENCE(3)"),
            (0, 15, 6, "=A1:A3"),
            (0, 19, 7, "=H19+1"),
            (0, 19, 8, "=G19+1"),
            (0, 1, 5, "=SUM(A1:A5)"),
            (0, 2, 5, "=E1+A6"),
            (0, 3, 5, "=E2*2"),
            (0, 4, 5, "=E3&\"z\""),
            (1, 1, 4, "=SUM(Sheet1!A1:A5)"),
            (1, 2, 4, "=Sheet1!E1"),
        ];
        for (sheet, row, col, f) in fixed {
            if !self.template_allowed(f) {
                continue;
            }
            self.push(Op::Set {
                sheet,
                row,
                col,
                value: f.to_string(),
            });
        }
        for row in 1..=FORMULA_ROWS {
            for col in FORMULA_COL_FIRST..=FORMULA_COL_LAST {
                let taken = self
                    .shadow
                    .get_cell_formula(0, row, col)
                    .ok()
                    .flatten()
                    .is_some();
                if taken || !self.rng.chance(55) {
                    continue;
                }
                let f = self.random_formula(0);
                self.push(Op::Set {
                    sheet: 0,
                    row,
                    col,
                    value: f,
                });
            }
        }
        for row in 1..=6 {
            for col in 4..=5 {
                let taken = self
                    .shadow
                    .get_cell_formula(1, row, col)
                    .ok()
                    .flatten()
                    .is_some();
                if taken || !self.rng.chance(70) {
                    continue;
                }
                let f = self.random_formula(1);
                self.push(Op::Set {
                    sheet: 1,
                    row,
                    col,
                    value: f,
                });
            }
        }
        if self.rng.chance(50) {
            self.push(Op::ArrayFormula {
                sheet: 0,
                row: 17,
                col: 7,
                width: 1,
                height: 2,
                formula: "=A1:A2*2".into(),
            });
        }
        self.push(Op::Evaluate);
    }

    /// One random mutation (not including the Evaluate).
    pub fn random_op(&mut self) {
        let roll = self.rng.below(1000);
        let op = if roll < 300 {
            // value edit on data region
            let sheet = self.random_sheet();
            let row = self.rng.range(1, DATA_ROWS);
            let col = self.rng.range(1, if sheet == 0 { DATA_COLS } else { 3 });
            let value = self.random_value();
            match self.rng.below(10) {
                0 => Op::SetNumber {
                    sheet,
                    row,
                    col,
                    value: self.rng.range(-5, 120) as f64,
                },
                1 => Op::SetText {
                    sheet,
                    row,
                    col,
                    value: "t".to_string(),
                },
                2 => Op::SetBool {
                    sheet,
                    row,
                    col,
                    value: self.rng.chance(50),
                },
                _ => Op::Set {
                    sheet,
                    row,
                    col,
                    value,
                },
            }
        } else if roll < 390 {
            // overwrite a formula cell with a value
            let cells = self.formula_cells();
            if cells.is_empty() {
                return;
            }
            let (sheet, row, col) = *self.rng.pick(&cells);
            let value = self.random_value();
            match self.rng.below(6) {
                0 => Op::SetNumber {
                    sheet,
                    row,
                    col,
                    value: self.rng.range(0, 50) as f64,
                },
                1 => Op::SetText {
                    sheet,
                    row,
                    col,
                    value: "ov".to_string(),
                },
                _ => Op::Set {
                    sheet,
                    row,
                    col,
                    value,
                },
            }
        } else if roll < 490 {
            // formula edit (formula region, occasionally a data cell)
            let sheet = self.random_sheet();
            let (row, col) = if self.rng.chance(15) {
                (self.rng.range(1, DATA_ROWS), self.rng.range(1, DATA_COLS))
            } else if sheet == 0 {
                (
                    self.rng.range(1, FORMULA_ROWS),
                    self.rng.range(FORMULA_COL_FIRST, FORMULA_COL_LAST),
                )
            } else {
                (self.rng.range(1, 8), self.rng.range(4, 5))
            };
            let value = self.random_formula(sheet.min(1));
            Op::Set {
                sheet,
                row,
                col,
                value,
            }
        } else if roll < 550 {
            let sheet = self.random_sheet();
            if self.max_row(sheet) > self.cfg.max_rows {
                return;
            }
            Op::InsertRows {
                sheet,
                row: self.rng.range(1, 22),
                count: self.rng.range(1, 2),
            }
        } else if roll < 600 {
            let sheet = self.random_sheet();
            Op::DeleteRows {
                sheet,
                row: self.rng.range(1, 22),
                count: self.rng.range(1, 2),
            }
        } else if roll < 625 {
            let sheet = self.random_sheet();
            Op::InsertCols {
                sheet,
                col: self.rng.range(1, 8),
                count: 1,
            }
        } else if roll < 650 {
            let sheet = self.random_sheet();
            Op::DeleteCols {
                sheet,
                col: self.rng.range(1, 8),
                count: 1,
            }
        } else if roll < 690 {
            let sheet = self.random_sheet();
            if self.rng.chance(50) {
                Op::HideRow {
                    sheet,
                    row: self.rng.range(1, 14),
                    hidden: self.rng.chance(60),
                }
            } else {
                Op::HideCol {
                    sheet,
                    col: self.rng.range(1, 8),
                    hidden: self.rng.chance(60),
                }
            }
        } else if roll < 715 {
            let sheet = self.random_sheet();
            Op::CellStyle {
                sheet,
                row: self.rng.range(1, FORMULA_ROWS),
                col: self.rng.range(1, FORMULA_COL_LAST),
                variant: self.rng.below(4) as u8,
            }
        } else if roll < 760 {
            let (name, scope) = *self.rng.pick(NAME_POOL);
            let target = *self.rng.pick(NAME_TARGETS);
            let formula = self.fill(target);
            match self.rng.below(3) {
                0 => Op::NewName {
                    name: name.into(),
                    scope,
                    formula,
                },
                1 => {
                    let (new_name, new_scope) = if self.rng.chance(75) {
                        (name.to_string(), scope)
                    } else {
                        let (n, s) = *self.rng.pick(NAME_POOL);
                        (n.to_string(), s)
                    };
                    Op::UpdateName {
                        name: name.into(),
                        scope,
                        new_name,
                        new_scope,
                        formula,
                    }
                }
                _ => Op::DeleteName {
                    name: name.into(),
                    scope,
                },
            }
        } else if roll < 785 {
            let n = self.sheet_names().len() as u32;
            match self.rng.below(4) {
                0 if n < 3 => Op::AddSheet { name: "Tmp".into() },
                1 if n >= 3 => Op::DeleteSheet { index: n - 1 },
                2 => {
                    let cur = self.data_sheet_name();
                    Op::RenameSheet {
                        index: 1,
                        name: if cur == "Data" {
                            "Data2".into()
                        } else {
                            "Data".into()
                        },
                    }
                }
                _ => Op::AddSheet { name: "Tmp".into() },
            }
        } else if roll < 815 {
            let sheet = self.random_sheet();
            if self.rng.chance(50) {
                Op::MoveRows {
                    sheet,
                    row: self.rng.range(1, 20),
                    count: self.rng.range(1, 2),
                    delta: *self.rng.pick(&[-3, -1, 1, 2, 4]),
                }
            } else {
                Op::MoveCols {
                    sheet,
                    col: self.rng.range(1, 7),
                    count: 1,
                    delta: *self.rng.pick(&[-2, -1, 1, 2]),
                }
            }
        } else if roll < 850 {
            let sheet = self.random_sheet();
            let a = (
                sheet,
                self.rng.range(1, FORMULA_ROWS),
                self.rng.range(1, FORMULA_COL_LAST),
                self.rng.range(1, 3),
                self.rng.range(1, 3),
            );
            if self.rng.chance(50) {
                Op::ClearContents { area: a }
            } else {
                Op::ClearAll { area: a }
            }
        } else if roll < 870 {
            let sheet = 0;
            let row = self.rng.range(1, FORMULA_ROWS - 2);
            let col = self.rng.range(FORMULA_COL_FIRST, FORMULA_COL_LAST);
            let f = *self
                .rng
                .pick(&["=A1:A2*2", "=A1:A3+1", "=SUM(A1:A3)", "=B1:B2"]);
            Op::ArrayFormula {
                sheet,
                row,
                col,
                width: 1,
                height: self.rng.range(1, 3),
                formula: f.to_string(),
            }
        } else {
            // redundant evaluate
            Op::Evaluate
        };
        self.push(op);
    }

    pub fn generate(mut self) -> Vec<Op> {
        self.setup();
        let steps = self.cfg.steps;
        for _ in 0..steps {
            self.random_op();
            // batch a few edits before evaluating sometimes
            if self.rng.chance(70) {
                self.push(Op::Evaluate);
            }
        }
        self.push(Op::Evaluate);
        self.ops
    }
}

pub fn generate(seed: u64, cfg: GenConfig) -> Vec<Op> {
    Generator::new(seed, cfg).generate()
}

/// A stable signature for a failure so distinct bugs can be counted.
pub fn signature(f: &Failure) -> String {
    // kind + formula text if present
    let formula = f
        .detail
        .find("formula=")
        .map(|i| {
            let rest = &f.detail[i + 8..];
            let end = rest
                .find(": ")
                .or_else(|| rest.find(" is in"))
                .or_else(|| rest.find(" changed"))
                .unwrap_or(rest.len());
            rest[..end].to_string()
        })
        .unwrap_or_default();
    format!("{}|{}", f.kind, formula)
}

/// Asserts a scenario runs clean; panics with the failure and the op list
/// otherwise (used by repro tests).
pub fn assert_clean(ops: &[Op]) {
    if let Err(f) = run_scenario(ops, true, cfg!(feature = "recalc_verify")) {
        panic!("{f}\nops:\n{}", ops_to_rust(ops));
    }
}
