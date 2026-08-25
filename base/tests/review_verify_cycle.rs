#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[cfg(feature = "recalc_verify")]
use ironcalc_base::{Model, RecalcMode};
#[test]
#[cfg(feature = "recalc_verify")]
fn count_range_cycle_under_verify() {
    let mut x = Model::new_empty("t", "en", "UTC", "en")
        .unwrap()
        .with_recalc_mode(RecalcMode::Verify);
    x.set_user_input(0, 1, 1, "=COUNT(B1:B2)".to_string())
        .unwrap();
    x.set_user_input(0, 1, 2, "=COUNT(A1:A2)".to_string())
        .unwrap();
    x.evaluate();
    x.set_user_input(0, 5, 5, "1".to_string()).unwrap();
    x.evaluate();
    println!(
        "A1={:?} B1={:?}",
        x.get_cell_value_by_index(0, 1, 1),
        x.get_cell_value_by_index(0, 1, 2)
    );
}
