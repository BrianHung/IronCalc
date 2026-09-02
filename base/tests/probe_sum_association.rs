//! Composed aggregates must be bit-identical to default Full's direct scan.
//! The catastrophic-cancellation shape below differed by 2.1 (4.1 vs 2.0)
//! when composition combined per-row subtotals instead of continuing the
//! row-major fold.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ironcalc_base::{Model, RecalcMode};

fn run(mode: RecalcMode, formulas: &[(i32, i32, &str)]) -> Vec<String> {
    let mut x = Model::new_empty("t", "en", "UTC", "en")
        .unwrap()
        .with_recalc_mode(mode);
    let data = [
        (1, 1, "0.1"),
        (1, 2, "0.2"),
        (2, 1, "0.3"),
        (2, 2, "0.7"),
        (3, 1, "10000000000000000"),
        (3, 2, "1"),
        (4, 1, "-10000000000000000"),
        (4, 2, "0.1"),
    ];
    for (r, c, v) in data {
        x.set_user_input(0, r, c, v.to_string()).unwrap();
    }
    for (r, c, f) in formulas {
        x.set_user_input(0, *r, *c, f.to_string()).unwrap();
    }
    x.evaluate();
    // A second pass after an edit exercises the cached-prefix path.
    x.set_user_input(0, 1, 1, "0.1".to_string()).unwrap();
    x.evaluate();
    formulas
        .iter()
        .map(|(r, c, _)| format!("{:?}", x.get_cell_value_by_index(0, *r, *c).unwrap()))
        .collect()
}

#[test]
fn composed_aggregates_are_bit_identical_to_full() {
    let formulas: &[(i32, i32, &str)] = &[
        // Overlapping prefixes: the longer ranges compose from the shorter.
        (10, 1, "=SUM(A1:B2)"),
        (11, 1, "=SUM(A1:B3)"),
        (12, 1, "=SUM(A1:B4)"),
        // Non-identity accumulator: must stream exactly like Full.
        (13, 1, "=SUM(5,A1:B4)"),
        (14, 1, "=MIN(A1:B4)"),
        (15, 1, "=MAX(A1:B4)"),
        (16, 1, "=COUNT(A1:B4)"),
    ];
    let full = run(RecalcMode::Full, formulas);
    let incremental = run(RecalcMode::Incremental, formulas);
    for (i, (f, inc)) in full.iter().zip(&incremental).enumerate() {
        assert_eq!(f, inc, "formula {:?} diverged", formulas[i].2);
    }
    println!("full == incremental: {full:?}");
}
