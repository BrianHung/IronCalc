//! The one cost invariant of the incremental engine that no value assertion can
//! express, and the reason it is not in the lib suite: it builds a 32k-cell
//! workbook and runs 600 passes over it, which the nightly mutation job would
//! pay for once per mutant. What else earns a place outside the lib suite is
//! `base/src/recalc/README.md`, "Test discipline".
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ironcalc_base::{Model, RecalcMode};
use std::time::Instant;

/// The cost of an incremental pass must depend on the size of the cone, not on
/// the number of cells in the workbook. Any whole-workbook walk per pass makes
/// this ratio grow with size while every value stays correct, so nothing else
/// in the suite can see it. Killed by calling `collect_array_cells` from
/// `evaluate_selective`.
#[test]
fn pass_cost_does_not_grow_with_workbook_size() {
    let cost = |unrelated: i32| -> u128 {
        let mut model = Model::new_empty("t", "en", "UTC", "en")
            .unwrap()
            .with_recalc_mode(RecalcMode::Incremental);
        // A 200-cell chain in column A: the cone for an A1 edit.
        model.set_user_input(0, 1, 1, "1".to_string()).unwrap();
        for row in 2..=200 {
            model
                .set_user_input(0, row, 1, format!("=A{}+1", row - 1))
                .unwrap();
        }
        // Unrelated formulas, far away, never in the cone.
        for i in 0..unrelated {
            model
                .set_user_input(0, 1000 + i / 20, 10 + i % 20, "=1+0".to_string())
                .unwrap();
        }
        model.evaluate();
        let start = Instant::now();
        for i in 0..300 {
            model
                .set_user_input(0, 1, 1, format!("{}", i % 97))
                .unwrap();
            model.evaluate();
        }
        start.elapsed().as_micros()
    };
    let small = cost(2_000).max(1);
    let large = cost(32_000);
    println!("300 passes over a 200-cell cone: 2k cells {small}us, 32k cells {large}us");
    assert!(
        large < small * 3 + 200_000,
        "pass cost grows with workbook size: 2k={small}us 32k={large}us"
    );
}

/// An incremental pass may cost less than a full one, or the same; it may never
/// cost dramatically *more*. Both modes walk the same clipped range for these
/// aggregates, so the only thing incremental adds is the bookkeeping -- and the
/// bookkeeping must be charged per range read, not per cell of the range.
///
/// The mutant is dropping the widening in `ReadSet::record_input`, which makes
/// `SUBTOTAL`'s per-row hidden test and per-cell subtotal test an input each:
/// the linear dedup that keeps the read set unique turns quadratic in the used
/// range, and this ratio goes from about 1x to two orders of magnitude. The
/// bound is a 10x margin over `Full` measured on the same machine in the same
/// run, plus a floor, so it is nowhere near the noise and still far under what
/// the per-cell form costs.
#[test]
fn incremental_costs_no_more_than_full_over_whole_column_aggregates() {
    let cost = |mode: RecalcMode| -> u128 {
        let mut model = Model::new_empty("t", "en", "UTC", "en")
            .unwrap()
            .with_recalc_mode(mode);
        for row in 1..=2_000 {
            model
                .set_user_input(0, row, 1, format!("{}", row % 89 + 1))
                .unwrap();
        }
        // One of each walk: the fold, the criteria walk, the clipped value
        // walks, and SUBTOTAL's hidden-row walk.
        for (i, formula) in [
            "=SUM(A:A)",
            "=COUNTIF(A:A,\">3\")",
            "=MAX(A:A)",
            "=AVERAGE(A:A)",
            "=COUNTA(A:A)",
            "=SUBTOTAL(103,A:A)",
        ]
        .iter()
        .enumerate()
        {
            model
                .set_user_input(0, 1, 3 + i as i32, (*formula).to_string())
                .unwrap();
        }
        model.evaluate();
        let start = Instant::now();
        for i in 0..20 {
            model
                .set_user_input(0, 1_000, 1, format!("{}", i % 89 + 1))
                .unwrap();
            model.evaluate();
        }
        start.elapsed().as_micros()
    };
    let full = cost(RecalcMode::Full).max(1);
    let incremental = cost(RecalcMode::Incremental);
    println!("20 passes over whole-column aggregates: full {full}us, incremental {incremental}us");
    assert!(
        incremental < full * 10 + 200_000,
        "an incremental pass costs more than the full pass it replaces: \
         full={full}us incremental={incremental}us"
    );
}

/// A whole-column reference spans 1,048,576 rows and a whole-row reference
/// 16,384 columns, but everything past the sheet's used range is blank. An
/// aggregate that ignores blanks must therefore cost what the same aggregate
/// over the used range costs -- and nothing in the value suite can see the
/// difference, because walking the blanks gives exactly the same answer.
///
/// The mutant is "walk the declared extent": drop the `clip_range_to_used` call
/// from any of these six walks and this workbook goes from about a millisecond
/// an evaluate to a quarter of a second. The bound is a 100x margin over the
/// bounded-range form measured on the same machine in the same run, plus a
/// floor, so it is nowhere near the noise and still two orders of magnitude
/// under the unclipped cost.
#[test]
fn whole_column_aggregate_cost_tracks_the_used_range() {
    let cost = |aggregates: &str| -> u128 {
        let mut model = Model::new_empty("t", "en", "UTC", "en")
            .unwrap()
            .with_recalc_mode(RecalcMode::Incremental);
        // A 40x4 used range: four orders of magnitude short of the sheet.
        for row in 1..=40 {
            for column in 1..=4 {
                model
                    .set_user_input(0, row, column, format!("{}", row * column))
                    .unwrap();
            }
        }
        model
            .set_user_input(0, 1, 8, aggregates.to_string())
            .unwrap();
        model.evaluate();
        let start = Instant::now();
        for i in 0..60 {
            model
                .set_user_input(0, 1, 1, format!("{}", i % 7 + 1))
                .unwrap();
            model.evaluate();
        }
        start.elapsed().as_micros()
    };
    // Six walks that would run to LAST_ROW/LAST_COLUMN unclipped, against the
    // same six over the used range. COUNTBLANK is in both because its clip is
    // the one that has to add a remainder back.
    let bounded = cost(
        "=COUNTA(A1:A40)+COUNTBLANK(B1:B40)+AVERAGE(A1:A40)\
         +SUBTOTAL(103,A1:A40)+MAX(A1:A40)+COUNT(A1:D5)",
    )
    .max(1);
    let open = cost(
        "=COUNTA(A:A)+COUNTBLANK(B:B)+AVERAGE(A:A)\
         +SUBTOTAL(103,A:A)+MAX(A:A)+COUNT(1:5)",
    );
    println!("60 passes: bounded ranges {bounded}us, whole column/row {open}us");
    assert!(
        open < bounded * 100 + 200_000,
        "a whole-column aggregate costs more than its used range: \
         bounded={bounded}us open={open}us"
    );
}
