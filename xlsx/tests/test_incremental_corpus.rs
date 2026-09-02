//! Real-workbook end-to-end proof for the incremental engine.
//!
//! The clause: for every workbook in `tests/calc_tests/*.xlsx`, a `Full` model
//! and an `Incremental` model behave identically across the whole
//! import -> evaluate -> edit -> export lifecycle. This crate owns xlsx import,
//! so this is the only place the claim can be made against real files rather
//! than synthesized ones.
//!
//! Each workbook is imported twice, evaluated in both modes, then driven
//! through the same scripted edit battery in lockstep. Every cell (value, type,
//! formatted text and formula) is compared after every `evaluate()`, and the two
//! models are finally exported to bytes and re-imported so the written output is
//! compared semantically (the raw zip bytes carry a save timestamp and can never
//! be equal).
//!
//! The battery is derived from each workbook's own content -- its numeric input
//! cells and its used range -- so it is meaningful per file without any
//! per-workbook special casing. A workbook that fails to import is skipped with
//! a logged reason; a workbook that diverges fails the test naming the file, the
//! op index and the cell.
//!
//! What this reaches, measured by mutation: neutering the dirty-marking of an
//! edited cell, or making a range read propagate from its first row only, are
//! both caught on five workbooks apiece. The graph's *structural* bookkeeping is
//! not reached -- deleting the row-band edge shift or the delete-shrink guard
//! leaves the corpus green, because `insert_rows`/`delete_rows` journal every
//! cell they move and the graph rebuilds those nodes from the journal anyway.
//! Displacement bookkeeping therefore stays the differential fuzzer's job
//! (`base/tests/fuzz_differential.rs`); this test's contribution is the real
//! import -> edit -> export lifecycle, which the fuzzer's synthesized models
//! never exercise.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::Cursor;

use ironcalc::base::cell::CellValue;
use ironcalc::base::expressions::utils::number_to_column;
use ironcalc::base::{ChangedSinceRead, Model, RecalcMode};
use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::{load_from_xlsx, load_from_xlsx_bytes};
use ironcalc::util::get_workbook_metadata;

const CORPUS: &str = "tests/calc_tests";
/// How many numeric input cells the battery overwrites. Kept small so the whole
/// corpus stays a couple of seconds in a debug build.
const OVERWRITES: usize = 4;

// ---------------------------------------------------------------------------
// Observable state
// ---------------------------------------------------------------------------

type Pos = (u32, i32, i32);

/// Everything about a cell the two modes must agree on.
#[derive(PartialEq)]
struct CellSnap {
    value: String,
    ty: String,
    formatted: String,
    formula: Option<String>,
}

struct Snapshot {
    sheets: Vec<String>,
    cells: BTreeMap<Pos, CellSnap>,
}

fn snapshot(model: &Model) -> Snapshot {
    let sheets = model
        .get_worksheets_properties()
        .into_iter()
        .map(|p| p.name)
        .collect();
    let mut cells = BTreeMap::new();
    for c in model.get_all_cells() {
        cells.insert(
            (c.index, c.row, c.column),
            CellSnap {
                value: format!(
                    "{:?}",
                    model.get_cell_value_by_index(c.index, c.row, c.column)
                ),
                ty: format!("{:?}", model.get_cell_type(c.index, c.row, c.column)),
                formatted: model
                    .get_formatted_cell_value(c.index, c.row, c.column)
                    .unwrap_or_else(|e| format!("ERR<{e}>")),
                formula: model
                    .get_cell_formula(c.index, c.row, c.column)
                    .ok()
                    .flatten(),
            },
        );
    }
    Snapshot { sheets, cells }
}

fn cell_ref(sheets: &[String], (sheet, row, column): Pos) -> String {
    let name = sheets
        .get(sheet as usize)
        .cloned()
        .unwrap_or_else(|| format!("#{sheet}"));
    let col = number_to_column(column).unwrap_or_else(|| format!("<col {column}>"));
    format!("{name}!{col}{row}")
}

/// The first way the two snapshots differ, or `None` if they are identical.
fn first_divergence(full: &Snapshot, inc: &Snapshot) -> Option<String> {
    if full.sheets != inc.sheets {
        return Some(format!(
            "sheet list: full={:?} incremental={:?}",
            full.sheets, inc.sheets
        ));
    }
    for pos in full.cells.keys().chain(inc.cells.keys()) {
        let at = cell_ref(&full.sheets, *pos);
        match (full.cells.get(pos), inc.cells.get(pos)) {
            (Some(a), Some(b)) => {
                if a.value != b.value {
                    return Some(format!(
                        "{at} value: full={} incremental={}",
                        a.value, b.value
                    ));
                }
                if a.ty != b.ty {
                    return Some(format!("{at} type: full={} incremental={}", a.ty, b.ty));
                }
                if a.formatted != b.formatted {
                    return Some(format!(
                        "{at} formatted: full={:?} incremental={:?}",
                        a.formatted, b.formatted
                    ));
                }
                if a.formula != b.formula {
                    return Some(format!(
                        "{at} formula: full={:?} incremental={:?}",
                        a.formula, b.formula
                    ));
                }
            }
            (Some(_), None) => return Some(format!("{at}: present in full only")),
            (None, Some(_)) => return Some(format!("{at}: present in incremental only")),
            (None, None) => unreachable!("position came from one of the two maps"),
        }
    }
    None
}

// ---------------------------------------------------------------------------
// The edit battery
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Op {
    Set {
        row: i32,
        column: i32,
        value: String,
    },
    InsertRow {
        row: i32,
    },
    DeleteRow {
        row: i32,
    },
    Evaluate,
}

fn apply(model: &mut Model, sheet: u32, op: &Op) -> String {
    let result = match op {
        Op::Set { row, column, value } => model.set_user_input(sheet, *row, *column, value.clone()),
        Op::InsertRow { row } => model.insert_rows(sheet, *row, 1),
        Op::DeleteRow { row } => model.delete_rows(sheet, *row, 1),
        Op::Evaluate => {
            model.evaluate();
            Ok(())
        }
    };
    format!("{result:?}")
}

/// What the battery is derived from: the sheet's own numeric input cells and
/// its used range.
struct Content {
    /// Cells with no formula holding a number, in reading order.
    inputs: Vec<(i32, i32)>,
    min_row: i32,
    max_row: i32,
    min_col: i32,
    max_col: i32,
}

fn content(model: &Model, sheet: u32) -> Content {
    let mut inputs = Vec::new();
    let (mut min_row, mut max_row, mut min_col, mut max_col) = (i32::MAX, 0, i32::MAX, 0);
    for c in model.get_all_cells() {
        if c.index != sheet {
            continue;
        }
        min_row = min_row.min(c.row);
        max_row = max_row.max(c.row);
        min_col = min_col.min(c.column);
        max_col = max_col.max(c.column);
        let is_input = model
            .get_cell_formula(c.index, c.row, c.column)
            .ok()
            .flatten()
            .is_none();
        if is_input
            && matches!(
                model.get_cell_value_by_index(c.index, c.row, c.column),
                Ok(CellValue::Number(_))
            )
        {
            inputs.push((c.row, c.column));
        }
    }
    inputs.sort_unstable();
    // An empty sheet still gets the structural half of the battery.
    if min_row == i32::MAX {
        (min_row, max_row, min_col, max_col) = (1, 1, 1, 1);
    }
    Content {
        inputs,
        min_row,
        max_row,
        min_col,
        max_col,
    }
}

/// Overwrites the first `OVERWRITES` numeric input cells with fresh values.
/// `nonce` separates one such group from another so the second round really
/// changes the values the first round wrote.
fn overwrite(inputs: &[(i32, i32)], nonce: f64) -> Vec<Op> {
    inputs
        .iter()
        .take(OVERWRITES)
        .enumerate()
        .map(|(i, (row, column))| Op::Set {
            row: *row,
            column: *column,
            value: format!("{}", 7.0 * (i + 1) as f64 + nonce),
        })
        .collect()
}

/// Two evaluates end every group: the first is the dirty pass, the second is a
/// no-op pass that must not move a cell.
fn group(ops: &mut Vec<Op>, mut new: Vec<Op>) {
    ops.append(&mut new);
    ops.push(Op::Evaluate);
    ops.push(Op::Evaluate);
}

/// Builds the battery from the workbook's own content: overwrite the first
/// `OVERWRITES` numeric input cells, clear one, insert a row before the middle
/// of the used range, delete the first row of the used range, then add formulas
/// that reference the existing cells.
///
/// The insert and the delete are deliberately at different rows. An insert at
/// `middle` undone by a delete at `middle + 1` would leave every surviving row
/// back at its original index, and a graph whose edges were shifted twice by
/// equal and opposite amounts is indistinguishable from one never shifted at
/// all -- the displacement would be tested against itself. Deleting the top of
/// the used range instead leaves a real net displacement on every row above
/// `middle`, which the trailing edit round in `run_workbook` then reads back
/// through.
fn battery(model: &Model, sheet: u32) -> Vec<Op> {
    let Content {
        inputs,
        min_row,
        max_row,
        min_col,
        max_col,
    } = content(model, sheet);
    let mut ops = Vec::new();

    group(&mut ops, overwrite(&inputs, 0.5));

    let clear = inputs
        .get(OVERWRITES)
        .or_else(|| inputs.first())
        .map(|(row, column)| Op::Set {
            row: *row,
            column: *column,
            value: String::new(),
        });
    group(&mut ops, clear.into_iter().collect());

    let middle = min_row + (max_row - min_row) / 2;
    group(&mut ops, vec![Op::InsertRow { row: middle }]);
    group(&mut ops, vec![Op::DeleteRow { row: min_row }]);

    let first = number_to_column(min_col).unwrap();
    let last = number_to_column(max_col).unwrap();
    // Two rows clear of the used range, so the new formulas cannot be circular.
    let at = max_row + 2;
    group(
        &mut ops,
        vec![
            Op::Set {
                row: at,
                column: min_col,
                value: format!("=SUM({first}{min_row}:{last}{max_row})"),
            },
            Op::Set {
                row: at + 1,
                column: min_col,
                value: format!("=COUNT({first}{min_row}:{last}{max_row})+N({first}{min_row})"),
            },
        ],
    );

    ops
}

// ---------------------------------------------------------------------------
// The proof
// ---------------------------------------------------------------------------

/// Round-trips a model through the exporter and back, so two exports can be
/// compared by content: the zip bytes themselves embed a save timestamp.
fn export_and_reimport(model: &Model, name: &str, locale: &str) -> Model<'static> {
    let bytes = save_xlsx_to_writer(model, Cursor::new(Vec::new()))
        .expect("export failed")
        .into_inner();
    let workbook = load_from_xlsx_bytes(&bytes, name, locale, "UTC").expect("re-import failed");
    Model::from_workbook(workbook, "en").expect("model from re-imported workbook")
}

/// What one workbook's run measured.
struct Run {
    cells: usize,
    ops: usize,
    /// Evaluates on the Incremental model that reported a genuine cell delta
    /// rather than `Everything`, which is what says the incremental path was
    /// really taken and the agreement above is not vacuous.
    ///
    /// A lower bound, not an exact count: a pass right after a structural edit
    /// sets `structural_unknown` and so reports `Everything` even when the pass
    /// itself was incremental. Undercounting is the safe direction for a floor.
    incremental_passes: usize,
    evaluates: usize,
}

/// Runs one workbook, returning its measurements or the first divergence found.
fn run_workbook(path: &str, name: &str) -> Result<Run, String> {
    // Loading already evaluates some cells (conditional formatting), so the
    // clock is mocked from the workbook's own metadata before the models under
    // comparison are built -- the same order `compare::test_file` uses.
    let probe = load_from_xlsx(path, "en", "UTC", "en").map_err(|e| format!("{e:?}"))?;
    let locale = get_workbook_metadata(&probe);
    ironcalc::mock_time::set_mock_time_from_metadata(&probe);
    drop(probe);

    let load = |mode: RecalcMode| -> Result<Model<'static>, String> {
        load_from_xlsx(path, &locale, "UTC", "en")
            .map(|m| m.with_recalc_mode(mode))
            .map_err(|e| format!("{e:?}"))
    };
    let mut full = load(RecalcMode::Full)?;
    let mut inc = load(RecalcMode::Incremental)?;

    // The battery is read off the Full model, so both models are driven by the
    // same op list whatever the incremental engine does to its own state.
    let mut run = Run {
        cells: 0,
        ops: 0,
        incremental_passes: 0,
        evaluates: 0,
    };
    let check = |op: usize, full: &Model, inc: &mut Model, run: &mut Run| -> Result<(), String> {
        run.evaluates += 1;
        if let ChangedSinceRead::Cells(_) = inc.take_changed_cells() {
            run.incremental_passes += 1;
        }
        let (a, b) = (snapshot(full), snapshot(inc));
        run.cells += a.cells.len().max(b.cells.len());
        match first_divergence(&a, &b) {
            Some(d) => Err(format!("op {op}: {d}")),
            None => Ok(()),
        }
    };
    full.evaluate();
    inc.evaluate();
    check(0, &full, &mut inc, &mut run)?;

    let sheet = full
        .get_worksheets_properties()
        .iter()
        .position(|p| !p.name.eq_ignore_ascii_case("METADATA"))
        .unwrap_or(0) as u32;
    let replay =
        |ops: Vec<Op>, full: &mut Model, inc: &mut Model, run: &mut Run| -> Result<(), String> {
            for op in &ops {
                run.ops += 1;
                let index = run.ops;
                let a = apply(full, sheet, op);
                let b = apply(inc, sheet, op);
                if a != b {
                    return Err(format!(
                        "op {index} ({op:?}) returned full={a} incremental={b}"
                    ));
                }
                if matches!(op, Op::Evaluate) {
                    check(index, full, inc, run)?;
                }
            }
            Ok(())
        };
    replay(battery(&full, sheet), &mut full, &mut inc, &mut run)?;
    // One more edit round, planned against the *displaced* workbook. Everything
    // above edits cells at their pre-insert addresses; only an edit made after
    // the structural ops reads dependents back through graph edges the
    // displacement had to shift, which is the half of the lifecycle a
    // cancelling insert/delete pair would leave untested.
    let inputs = content(&full, sheet).inputs;
    replay(
        {
            let mut ops = Vec::new();
            group(&mut ops, overwrite(&inputs, 0.25));
            ops
        },
        &mut full,
        &mut inc,
        &mut run,
    )?;

    // The written output must agree too, compared after a re-import because the
    // zip bytes carry a save timestamp.
    let (a, b) = (
        snapshot(&export_and_reimport(&full, name, &locale)),
        snapshot(&export_and_reimport(&inc, name, &locale)),
    );
    run.cells += a.cells.len().max(b.cells.len());
    if let Some(d) = first_divergence(&a, &b) {
        return Err(format!("export: {d}"));
    }
    Ok(run)
}

#[test]
fn full_and_incremental_agree_on_every_calc_test_workbook() {
    let mut files: Vec<_> = std::fs::read_dir(CORPUS)
        .expect("corpus directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "xlsx"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no workbooks found in {CORPUS}");

    let started = std::time::Instant::now();
    let (mut skipped, mut failures) = (Vec::new(), Vec::new());
    let mut rows = Vec::new();
    let (mut incremental_passes, mut evaluates) = (0, 0);
    for path in &files {
        let display = path.display().to_string();
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        // A workbook this crate cannot import at all is a pre-existing import
        // gap, not an incremental-engine finding: log it and keep going.
        let importable = load_from_xlsx(&display, "en", "UTC", "en");
        if let Err(e) = importable {
            skipped.push(format!("{name}: import failed: {e:?}"));
            rows.push(format!(
                "{name:<32} {:>7} {:>5}  SKIPPED (import)",
                "-", "-"
            ));
            continue;
        }
        drop(importable);

        match run_workbook(&display, &name) {
            Ok(run) => {
                incremental_passes += run.incremental_passes;
                evaluates += run.evaluates;
                rows.push(format!(
                    "{name:<32} {:>7} {:>5} {:>10}  ok",
                    run.cells,
                    run.ops,
                    format!("{}/{}", run.incremental_passes, run.evaluates)
                ));
            }
            Err(e) => {
                rows.push(format!(
                    "{name:<32} {:>7} {:>5} {:>10}  FAILED",
                    "-", "-", "-"
                ));
                failures.push(format!("{name}: {e}"));
            }
        }
    }

    eprintln!(
        "\n==== full vs incremental over {} workbooks in {CORPUS} ====\n{:<32} {:>7} {:>5} {:>10}\n{}\n{} skipped, {} failed, {incremental_passes}/{evaluates} incremental passes, {:.1?} elapsed",
        files.len(),
        "workbook",
        "cells",
        "ops",
        "incr/eval",
        rows.join("\n"),
        skipped.len(),
        failures.len(),
        started.elapsed(),
    );
    for reason in &skipped {
        eprintln!("skipped: {reason}");
    }
    assert!(
        failures.is_empty(),
        "full and incremental diverged:\n  {}",
        failures.join("\n  ")
    );
    // Anti-vacuity: agreement proves nothing if every pass fell back to full.
    // A pass that reports a cell delta is one the incremental engine actually
    // ran. The floor is corpus-wide on purpose -- a per-workbook floor would be
    // a per-workbook exemption list the moment one file legitimately cannot go
    // incremental.
    assert!(
        incremental_passes * 2 >= evaluates,
        "only {incremental_passes} of {evaluates} evaluates took the incremental path; \
         the comparison above is mostly full-vs-full and proves little"
    );
}
