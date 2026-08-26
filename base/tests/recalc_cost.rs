//! The one cost invariant of the incremental engine that no value assertion can
//! express, and the reason it is not in the lib suite: it builds a 32k-cell
//! workbook and runs 600 passes over it, which the nightly mutation job would
//! pay for once per mutant.
//!
//! Everything else that used to live in `base/tests/` outside the fuzz harness
//! is now a lib test; see `base/src/recalc/README.md` "Test discipline".
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ironcalc_base::{Model, RecalcMode};
use std::time::Instant;

/// The cost of an incremental pass must depend on the size of the cone, not on
/// the number of cells in the workbook. Any whole-workbook walk per pass -- the
/// old post-pass `collect_array_cells` was one -- makes this ratio grow with
/// size while every value in the workbook stays correct, so nothing else in the
/// suite can see it. Killed by calling `collect_array_cells` from
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
