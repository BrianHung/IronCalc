#![allow(clippy::unwrap_used)]

use crate::{Model, RecalcMode};

/// Incremental evaluation produces the same values as full for a value edit that
/// cascades through dependents.
#[test]
fn incremental_matches_full_on_a_chain() {
    let mut model = Model::new_empty("m", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 1, "1".into()).unwrap(); // A1 = 1
    model.set_user_input(0, 1, 2, "=A1*2".into()).unwrap(); // B1 = 2
    model.set_user_input(0, 1, 3, "=B1+1".into()).unwrap(); // C1 = 3
    model.evaluate();
    model.set_recalc_mode(RecalcMode::Incremental);
    model.evaluate();

    model.set_user_input(0, 1, 1, "10".into()).unwrap(); // A1 = 10
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 1, 2).unwrap(), "20"); // B1
    assert_eq!(model.get_formatted_cell_value(0, 1, 3).unwrap(), "21"); // C1
}

/// Editing one independent chain does not disturb another.
#[test]
fn incremental_isolates_independent_chains() {
    let mut model = Model::new_empty("m", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 1, "1".into()).unwrap(); // A1
    model.set_user_input(0, 2, 1, "=A1+1".into()).unwrap(); // A2 = A1+1
    model.set_user_input(0, 1, 2, "100".into()).unwrap(); // B1 (independent)
    model.set_user_input(0, 2, 2, "=B1+1".into()).unwrap(); // B2 = B1+1
    model.evaluate();
    model.set_recalc_mode(RecalcMode::Incremental);
    model.evaluate();

    model.set_user_input(0, 1, 1, "5".into()).unwrap(); // edit A1 only
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 2, 1).unwrap(), "6"); // A2 updated
    assert_eq!(model.get_formatted_cell_value(0, 2, 2).unwrap(), "101"); // B2 unchanged
}
