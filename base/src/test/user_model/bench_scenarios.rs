//! Measured cost of a single edit on realistic workbook shapes, in both recalc
//! modes.
//!
//! This is measurement, not a test: nothing here asserts a wall-clock budget.
//! The one cost *invariant* -- that a pass costs the size of the cone and not
//! the size of the workbook -- is asserted in `base/tests/recalc_cost.rs`. What
//! this file produces is the evidence a reviewer needs to judge the trade: for
//! each shape, what a whole-workbook pass costs, what one edit costs in default
//! `Full` mode, and what the same edit costs in `Incremental`.
//!
//! The scenarios deliberately include the shapes incremental does *not* win on
//! -- a wide-fanout dashboard edit, a spill that forces the array fallback, a
//! workbook floor set by volatile cells, whole-column aggregates whose cost is
//! the range scan either way. A table that only showed the chains would be
//! marketing.
//!
//! Run (one test, so the scenarios never time each other's threads):
//!
//! ```text
//! cargo test -p ironcalc_base bench_scenarios --release -- --ignored --nocapture
//! ```
//!
//! The builders below use only pre-incremental APIs (`add_sheet`,
//! `set_user_input`, `insert_rows`, `move_rows_action`, `evaluate`) so the same
//! shapes can be built on a pre-stack tree to measure `Full` there and check
//! that the default mode did not regress.
#![allow(clippy::unwrap_used)]
#![allow(clippy::print_stdout)]

use crate::expressions::utils::number_to_column;
use crate::model::incremental::EvalPass;
use crate::{ChangedSinceRead, Model, RecalcMode};
use std::time::{Duration, Instant};

/// Untimed rounds before every measured series, so the first pass after a build
/// -- which warms allocators and the parse cache -- is not in the sample.
const WARMUP: usize = 5;

// ---------------------------------------------------------------------------
// Timing
// ---------------------------------------------------------------------------

/// Timings of one measured operation, kept unsorted until read.
struct Samples(Vec<Duration>);

impl Samples {
    fn sorted(&self) -> Vec<Duration> {
        let mut v = self.0.clone();
        v.sort_unstable();
        v
    }

    fn median(&self) -> Duration {
        let v = self.sorted();
        v[v.len() / 2]
    }

    /// Median, and the spread as the 10th and 90th percentile around it. Wide
    /// spread means the median is not to be trusted; it is printed for every
    /// row rather than only when it is flattering.
    fn cell(&self) -> String {
        let v = self.sorted();
        let p = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
        format!(
            "{} <sub>{}&ndash;{}</sub>",
            fmt(self.median()),
            fmt(p(0.1)),
            fmt(p(0.9))
        )
    }
}

/// Fixed three significant figures, so a column of times lines up by eye.
fn fmt(d: Duration) -> String {
    let us = d.as_secs_f64() * 1e6;
    if us >= 10_000.0 {
        format!("{:.0} ms", us / 1000.0)
    } else if us >= 1_000.0 {
        format!("{:.1} ms", us / 1000.0)
    } else if us >= 10.0 {
        format!("{us:.0} us")
    } else {
        format!("{us:.1} us")
    }
}

/// Times the three measured series round-robin, one pass of each per round,
/// after `WARMUP` untimed rounds.
///
/// Round-robin rather than three loops back to back: a run of this bench takes
/// the better part of a minute, over which CPU frequency and page cache drift
/// enough to move a 13 ms pass to 18 ms. Timed in sequence that drift lands on
/// whichever column is measured last and reads as a regression; interleaved it
/// lands on all three equally.
///
/// Only `evaluate` is timed. The edit that precedes it costs the same in both
/// modes, and what is being compared is the recompute.
fn interleave(
    full: &mut Model,
    incremental: &mut Model,
    iters: usize,
    edit: impl Fn(&mut Model, usize) + Copy,
) -> (Samples, Samples, Samples) {
    let mut full_pass = Vec::with_capacity(iters);
    let mut edit_full = Vec::with_capacity(iters);
    let mut edit_incremental = Vec::with_capacity(iters);
    for i in 0..iters + WARMUP {
        let start = Instant::now();
        full.evaluate();
        let no_edit = start.elapsed();

        edit(full, i);
        let start = Instant::now();
        full.evaluate();
        let with_edit = start.elapsed();

        edit(incremental, i);
        let start = Instant::now();
        incremental.evaluate();
        let incrementally = start.elapsed();

        if i >= WARMUP {
            full_pass.push(no_edit);
            edit_full.push(with_edit);
            edit_incremental.push(incrementally);
        }
    }
    (
        Samples(full_pass),
        Samples(edit_full),
        Samples(edit_incremental),
    )
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

/// One line of the results table.
struct Row {
    scenario: &'static str,
    shape: String,
    full_pass: Samples,
    edit_full: Samples,
    edit_incremental: Samples,
    /// What `take_changed_cells` reported for one representative edit.
    delta: String,
    /// Whether that pass stayed incremental or handed off to a full one.
    pass: &'static str,
}

impl Row {
    fn render(&self) -> String {
        let speedup =
            self.edit_full.median().as_secs_f64() / self.edit_incremental.median().as_secs_f64();
        format!(
            "| {} | {} | {} | {} | {} | {:.2}x | {} | {} |",
            self.scenario,
            self.shape,
            self.full_pass.cell(),
            self.edit_full.cell(),
            self.edit_incremental.cell(),
            speedup,
            self.delta,
            self.pass,
        )
    }
}

/// The delta an incremental pass reported, as a table cell. `Everything` means
/// "rescan the workbook": a full pass ran, or an insert/delete moved cells the
/// cone cannot name. It is not by itself evidence of a fallback -- the `pass`
/// column is.
fn delta_of(model: &mut Model) -> String {
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => "Everything".to_string(),
        ChangedSinceRead::Cells(cells) => format!("{}", cells.len()),
    }
}

/// Applies `edit` and runs one pass the way `Model::evaluate` does in
/// incremental mode, reporting which arm it took. The timed columns go through
/// the public `evaluate`; this only reads what those passes were doing, since
/// a fallback is otherwise indistinguishable from a slow incremental pass.
fn classify(model: &mut Model, edit: impl Fn(&mut Model, usize), i: usize) -> &'static str {
    edit(model, i);
    model.drain_write_journal();
    let mut evaluating = model.pause_journal();
    match evaluating.evaluate_selective() {
        EvalPass::Incremental => "incremental",
        EvalPass::Full => "**full fallback**",
    }
}

/// Builds the shape in both modes, times the three columns, and reads the
/// delta of one incremental edit. `edit` receives an iteration counter so it
/// can write a different value each round (writing the same value again would
/// let the change check stop the pass at the seed).
fn measure(
    scenario: &'static str,
    shape: String,
    iters: usize,
    build: impl Fn(&mut Model),
    edit: impl Fn(&mut Model, usize) + Copy,
    probe: (u32, i32, i32),
) -> Row {
    let make = |mode| {
        let mut model = Model::new_empty("bench", "en", "UTC", "en")
            .unwrap()
            .with_recalc_mode(mode);
        build(&mut model);
        model.evaluate();
        model
    };

    let mut full = make(RecalcMode::Full);
    let mut incremental = make(RecalcMode::Incremental);
    let (full_pass, edit_full, edit_incremental) =
        interleave(&mut full, &mut incremental, iters, edit);

    // One more edit on a drained record, so the delta belongs to a single pass.
    // The index continues the series the loop just ran, so the value is one the
    // previous pass did not already write: rewriting the same value is not a
    // change, and would report an empty delta for every scenario.
    let _ = incremental.take_changed_cells();
    let last = iters + WARMUP;
    let pass = classify(&mut incremental, edit, last);
    let delta = delta_of(&mut incremental);
    // Keep the full model in step so the probe below compares like with like.
    edit(&mut full, last);
    full.evaluate();

    // Sanity: the two modes agree at the far end of the cone. A wrong number is
    // not a fast number, and a silent divergence would make every row a lie.
    let (sheet, row, column) = probe;
    assert_eq!(
        full.get_formatted_cell_value(sheet, row, column),
        incremental.get_formatted_cell_value(sheet, row, column),
        "{scenario}: modes disagree at the probe cell"
    );

    Row {
        scenario,
        shape,
        full_pass,
        edit_full,
        edit_incremental,
        delta,
        pass,
    }
}

// ---------------------------------------------------------------------------
// Shape builders. Pre-incremental APIs only, so they port to a baseline tree.
// ---------------------------------------------------------------------------

fn col(n: i32) -> String {
    number_to_column(n).unwrap()
}

/// 10 department sheets of 2000 rows: an input column, a per-row factor, a
/// running total that references the row above, and a windowed `SUM` over the
/// rows behind it. A rollup sheet totals every department.
fn build_financial_model(model: &mut Model, sheets: u32, rows: i32) {
    for s in 0..sheets {
        if s > 0 {
            model.add_sheet(&format!("Dept{s}")).unwrap();
        }
        for r in 1..=rows {
            model
                .set_user_input(s, r, 1, format!("{}", r % 97 + 1))
                .unwrap();
            model
                .set_user_input(s, r, 2, format!("=A{r}*1.05"))
                .unwrap();
            if r == 1 {
                model.set_user_input(s, r, 3, "=B1".to_string()).unwrap();
            } else {
                model
                    .set_user_input(s, r, 3, format!("=C{}+B{r}", r - 1))
                    .unwrap();
            }
            let window_start = (r - 9).max(1);
            model
                .set_user_input(s, r, 4, format!("=SUM(B{window_start}:B{r})*0.5"))
                .unwrap();
        }
    }
    model.add_sheet("Rollup").unwrap();
    let rollup = sheets;
    for s in 0..sheets {
        let name = if s == 0 {
            "Sheet1".to_string()
        } else {
            format!("Dept{s}")
        };
        model
            .set_user_input(
                rollup,
                s as i32 + 1,
                1,
                format!("=SUM({name}!C1:C{rows})+SUM({name}!D1:D{rows})"),
            )
            .unwrap();
    }
    model
        .set_user_input(rollup, sheets as i32 + 2, 1, format!("=SUM(A1:A{sheets})"))
        .unwrap();
}

/// One assumptions sheet every formula in the workbook reads, through two
/// intermediate layers. Editing the first assumption reaches essentially the
/// whole workbook -- the shape incremental is expected to lose on.
fn build_dashboard(model: &mut Model, sheets: u32, width: i32, layer2: i32, layer3: i32) {
    for k in 1..=50 {
        model.set_user_input(0, k, 1, format!("{k}")).unwrap();
    }
    for s in 1..=sheets {
        model.add_sheet(&format!("Dash{s}")).unwrap();
        for c in 1..=width {
            let name = col(c);
            // Layer 1: every cell reads the shared assumption A1 plus its own.
            model
                .set_user_input(
                    s,
                    1,
                    c,
                    format!("=Sheet1!$A$1*Sheet1!A{}", (c - 1) % 50 + 1),
                )
                .unwrap();
            // Layer 2, then layer 3, each reading the layer above in its column.
            for r in 2..=1 + layer2 {
                model
                    .set_user_input(s, r, c, format!("={name}{}+1", r - 1))
                    .unwrap();
            }
            for r in 2 + layer2..=1 + layer2 + layer3 {
                model
                    .set_user_input(s, r, c, format!("={name}{}*1.01", r - 1))
                    .unwrap();
            }
        }
    }
}

/// `A1 -> A2 -> ... -> An`, the worst case for a head edit and the best case
/// for a tail edit.
fn build_long_chain(model: &mut Model, length: i32) {
    model.set_user_input(0, 1, 1, "1".to_string()).unwrap();
    for r in 2..=length {
        model
            .set_user_input(0, r, 1, format!("=A{}+1", r - 1))
            .unwrap();
    }
}

/// 50k cells, 10k of them formulas, arranged in independent 20-row blocks: four
/// data columns and a fifth that totals its row and adds the block's running
/// total. Editing a block head touches exactly that block.
fn build_sparse_workbook(model: &mut Model, rows: i32, block: i32) {
    for r in 1..=rows {
        for c in 1..=4 {
            model
                .set_user_input(0, r, c, format!("{}", (r * c) % 89 + 1))
                .unwrap();
        }
        if (r - 1) % block == 0 {
            model
                .set_user_input(0, r, 5, format!("=SUM(A{r}:D{r})"))
                .unwrap();
        } else {
            model
                .set_user_input(0, r, 5, format!("=SUM(A{r}:D{r})+E{}", r - 1))
                .unwrap();
        }
    }
}

/// A long data column and a handful of aggregates over the whole column. Every
/// one of these clips the reference to the used range, so the row measures the
/// cone and not the sheet's full height; `MAX`, `AVERAGE`, `COUNTA` and
/// `SUBTOTAL` used to walk all 1,048,576 rows and had to be left out of it.
fn build_whole_column(model: &mut Model, rows: i32, aggregates: i32) {
    for r in 1..=rows {
        model
            .set_user_input(0, r, 1, format!("{}", r % 89 + 1))
            .unwrap();
    }
    for i in 0..aggregates {
        model
            .set_user_input(0, i + 1, 3, format!("=SUM(A:A)+{i}"))
            .unwrap();
        model
            .set_user_input(0, i + 1, 4, format!("=COUNTIF(A:A,\">{}\")", i + 1))
            .unwrap();
        model
            .set_user_input(0, i + 1, 5, format!("=MAX(A:A)+{i}"))
            .unwrap();
        model
            .set_user_input(0, i + 1, 6, format!("=AVERAGE(A:A)+{i}"))
            .unwrap();
        model
            .set_user_input(0, i + 1, 7, format!("=COUNTA(A:A)+{i}"))
            .unwrap();
        model
            .set_user_input(0, i + 1, 8, format!("=SUBTOTAL(103,A:A)+{i}"))
            .unwrap();
    }
}

/// Dynamic-array anchors, each reading its own scalar input, plus a reader of
/// each spill. Any cone that reaches an array falls back to a full pass.
fn build_spill_heavy(model: &mut Model, spills: i32) {
    model.add_sheet("Spills").unwrap();
    for i in 0..spills {
        model
            .set_user_input(0, i + 1, 1, format!("{}", i + 1))
            .unwrap();
        let anchor = 1 + i * 3;
        let name = col(anchor);
        let formula = if i % 2 == 0 {
            format!("=SEQUENCE(10,1,Sheet1!A{},1)", i + 1)
        } else {
            format!("=SORT(SEQUENCE(10,1,Sheet1!A{},1),1,-1)", i + 1)
        };
        model.set_user_input(1, 1, anchor, formula).unwrap();
        model
            .set_user_input(1, 1, anchor + 1, format!("=SUM({name}1:{name}10)"))
            .unwrap();
    }
}

/// A three-cell cycle. Its members never serve a stored value, so every pass
/// re-seeds them dirty: this is the floor an unrelated edit pays forever.
fn build_cycle(model: &mut Model, sheet: u32) {
    model
        .set_user_input(sheet, 1, 20, "=U2+1".to_string())
        .unwrap();
    model
        .set_user_input(sheet, 2, 20, "=U3+1".to_string())
        .unwrap();
    model
        .set_user_input(sheet, 3, 20, "=U1+1".to_string())
        .unwrap();
}

// ---------------------------------------------------------------------------
// Scenario sizes
// ---------------------------------------------------------------------------

const FIN_SHEETS: u32 = 10;
const FIN_ROWS: i32 = 2_000;
const CHAIN: i32 = 20_000;
const SPARSE_ROWS: i32 = 10_000;
const SPARSE_BLOCK: i32 = 20;
const WHOLE_COL_ROWS: i32 = 30_000;
const WHOLE_COL_AGGS: i32 = 10; // x2 formulas (SUM and COUNTIF) = 20
const SPILLS: i32 = 200;

// ---------------------------------------------------------------------------
// The bench
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn bench_scenarios() {
    let mut rows = Vec::new();

    // financial-model: an input cell deep in the first sheet. The running total
    // and the windowed SUM below it are the cone; the other nine sheets are not.
    rows.push(measure(
        "financial-model",
        format!(
            "{FIN_SHEETS} sheets x {FIN_ROWS} rows, {} formulas",
            FIN_SHEETS as i32 * FIN_ROWS * 3 + FIN_SHEETS as i32 + 1
        ),
        20,
        |m| build_financial_model(m, FIN_SHEETS, FIN_ROWS),
        |m, i| {
            m.set_user_input(0, 1_000, 1, format!("{}", i % 97 + 1))
                .unwrap()
        },
        (FIN_SHEETS, 1, 1),
    ));

    // dashboard: one assumption every formula reads. The cone is the workbook,
    // and the fanout guard should hand the pass to full rather than pay
    // bookkeeping for nothing.
    rows.push(measure(
        "dashboard (wide fanout)",
        "50 assumptions, 5 sheets x 1020 = 5100 formulas".to_string(),
        20,
        |m| build_dashboard(m, 5, 20, 10, 40),
        |m, i| {
            m.set_user_input(0, 1, 1, format!("{}", i % 97 + 1))
                .unwrap()
        },
        (1, 51, 1),
    ));

    // long-chain, head: the cone is the whole chain.
    rows.push(measure(
        "long-chain, edit head",
        format!("{CHAIN}-cell chain, cone = {CHAIN}"),
        20,
        |m| build_long_chain(m, CHAIN),
        |m, i| {
            m.set_user_input(0, 1, 1, format!("{}", i % 97 + 1))
                .unwrap()
        },
        (0, CHAIN, 1),
    ));

    // long-chain, tail: same workbook, ten cells of cone.
    rows.push(measure(
        "long-chain, edit tail",
        format!("{CHAIN}-cell chain, cone = 10"),
        50,
        |m| build_long_chain(m, CHAIN),
        |m, i| {
            m.set_user_input(0, CHAIN - 10, 1, format!("{}", i % 97 + 1))
                .unwrap()
        },
        (0, CHAIN, 1),
    ));

    // sparse-workbook: the bread-and-butter edit -- one block of 20.
    rows.push(measure(
        "sparse-workbook",
        format!(
            "{} cells, {SPARSE_ROWS} formulas, cone = {SPARSE_BLOCK}",
            SPARSE_ROWS * 5
        ),
        50,
        |m| build_sparse_workbook(m, SPARSE_ROWS, SPARSE_BLOCK),
        |m, i| {
            m.set_user_input(0, 5_001, 1, format!("{}", i % 89 + 1))
                .unwrap()
        },
        (0, 5_020, 5),
    ));

    // whole-column aggregates: the cone is 20 formulas, but each of them
    // rescans 30k rows, so the scan is the cost in either mode.
    rows.push(measure(
        "whole-column aggregates",
        format!(
            "{WHOLE_COL_ROWS}-row column, {} SUM/COUNTIF(A:A)",
            WHOLE_COL_AGGS * 2
        ),
        20,
        |m| build_whole_column(m, WHOLE_COL_ROWS, WHOLE_COL_AGGS),
        |m, i| {
            m.set_user_input(0, 15_000, 1, format!("{}", i % 89 + 1))
                .unwrap()
        },
        (0, 1, 3),
    ));

    // volatile-mix: the same sparse workbook with 20 always-dirty cells. They
    // re-roll on every pass whatever the edit was, which is the floor.
    rows.push(measure(
        "volatile-mix",
        format!(
            "{} cells + 20 NOW()/RAND(), cone = {SPARSE_BLOCK}",
            SPARSE_ROWS * 5
        ),
        50,
        |m| {
            build_sparse_workbook(m, SPARSE_ROWS, SPARSE_BLOCK);
            for i in 0..20 {
                let f = if i % 2 == 0 { "=NOW()" } else { "=RAND()" };
                m.set_user_input(0, i + 1, 10, f.to_string()).unwrap();
            }
        },
        |m, i| {
            m.set_user_input(0, 5_001, 1, format!("{}", i % 89 + 1))
                .unwrap()
        },
        (0, 5_020, 5),
    ));

    // spill-heavy: a one-cell edit whose cone reaches a dynamic array. The
    // engine cannot order a spill incrementally and runs a full pass; this row
    // is what that fallback costs.
    rows.push(measure(
        "spill-heavy",
        format!("{SPILLS} SEQUENCE/SORT spills + {SPILLS} readers"),
        20,
        |m| build_spill_heavy(m, SPILLS),
        |m, i| {
            m.set_user_input(0, 1, 1, format!("{}", i % 89 + 1))
                .unwrap()
        },
        (1, 1, 2),
    ));

    // pathological-cycle: an unrelated edit still pays for the cycle, whose
    // members are re-seeded dirty on every pass because they never serve a
    // stored value.
    rows.push(measure(
        "pathological-cycle",
        format!("sparse-workbook + one 3-cell cycle, cone = {SPARSE_BLOCK} + 3"),
        50,
        |m| {
            build_sparse_workbook(m, SPARSE_ROWS, SPARSE_BLOCK);
            build_cycle(m, 0);
        },
        |m, i| {
            m.set_user_input(0, 5_001, 1, format!("{}", i % 89 + 1))
                .unwrap()
        },
        (0, 5_020, 5),
    ));

    rows.push(structural_row("structural: insert_rows", |m, _| {
        m.insert_rows(0, 1_000, 1).unwrap();
    }));
    rows.push(structural_row("structural: move_rows", |m, i| {
        // Alternate the direction so the model does not drift far from its
        // starting shape over the iterations.
        let delta = if i % 2 == 0 { 1 } else { -1 };
        m.move_rows_action(0, 1_000, 1, delta).unwrap();
    }));

    print_table(&rows);
}

/// The two structural edits share the financial model and differ only in the
/// mutation, which is applied where the value edits go in `measure`.
fn structural_row(scenario: &'static str, edit: impl Fn(&mut Model, usize) + Copy) -> Row {
    measure(
        scenario,
        format!("financial-model ({FIN_SHEETS} sheets x {FIN_ROWS} rows), 1 row"),
        10,
        |m| build_financial_model(m, FIN_SHEETS, FIN_ROWS),
        edit,
        (FIN_SHEETS, 1, 1),
    )
}

fn print_table(rows: &[Row]) {
    println!("\n<!-- machine: fill in from `uname -a` / `sysctl -n machdep.cpu.brand_string` -->");
    println!(
        "\n| scenario | shape | full evaluate | edit, Full mode | edit, Incremental | speedup | delta | pass |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    for row in rows {
        println!("{}", row.render());
    }
    println!("\nTimes are the median of the timed iterations with the 10th-90th percentile\nbeside them; `evaluate` alone is timed, the edit that precedes it is not.");
    println!("\nOn a `full fallback` row both timed columns are the same whole-workbook pass,\nso the speedup column is the reciprocal of what read tracing costs: incremental\nmode records what every formula reads even on a pass it runs in full.\n");
}
