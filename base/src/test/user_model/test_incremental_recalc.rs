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

/// A value edit unrelated to a spill stays incremental and leaves the spill
/// intact; an edit that feeds the spill still yields correct values.
#[test]
fn incremental_coexists_with_arrays() {
    let mut model = Model::new_empty("m", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 2, "1".into()).unwrap(); // B1
    model.set_user_input(0, 2, 2, "2".into()).unwrap(); // B2
    model.set_user_input(0, 1, 1, "=B1:B2".into()).unwrap(); // A1 spills A1:A2
    model.set_user_input(0, 1, 4, "10".into()).unwrap(); // D1 (independent)
    model.set_user_input(0, 2, 4, "=D1+1".into()).unwrap(); // D2
    model.evaluate();
    model.set_recalc_mode(RecalcMode::Incremental);
    model.evaluate();

    // Edit unrelated to the spill: D2 updates, the spill is untouched.
    model.set_user_input(0, 1, 4, "20".into()).unwrap();
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 2, 4).unwrap(), "21"); // D2
    assert_eq!(model.get_formatted_cell_value(0, 2, 1).unwrap(), "2"); // A2 spill intact

    // Edit that feeds the spill: falls back to full, still correct.
    model.set_user_input(0, 2, 2, "9".into()).unwrap(); // B2
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 2, 1).unwrap(), "9"); // A2 re-spilled
}

/// A row insert grows a straddling range: `SUM` is unchanged (new cells empty)
/// while `ROWS` reflects the larger range, so the size-sensitive dependent must
/// recompute incrementally.
#[test]
fn incremental_handles_row_insert() {
    let mut model = Model::new_empty("m", "en", "UTC", "en").unwrap();
    for row in 1..=5 {
        model.set_user_input(0, row, 1, row.to_string()).unwrap(); // A1..A5
    }
    model.set_user_input(0, 1, 3, "=SUM(A1:A5)".into()).unwrap(); // C1
    model
        .set_user_input(0, 1, 5, "=ROWS(A1:A5)".into())
        .unwrap(); // E1
    model.evaluate();
    model.set_recalc_mode(RecalcMode::Incremental);
    model.evaluate();

    model.insert_rows(0, 3, 2).unwrap(); // A1:A5 -> A1:A7
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 1, 3).unwrap(), "15"); // SUM unchanged
    assert_eq!(model.get_formatted_cell_value(0, 1, 5).unwrap(), "7"); // ROWS grew
}

/// A delete with no tracked range stays incremental: the referencing cell shifts
/// up and its precedent, above the deletion, is untouched.
#[test]
fn incremental_handles_row_delete() {
    let mut model = Model::new_empty("m", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 1, "10".into()).unwrap(); // A1
    model.set_user_input(0, 5, 1, "=A1+1".into()).unwrap(); // A5
    model.evaluate();
    model.set_recalc_mode(RecalcMode::Incremental);
    model.evaluate();

    model.delete_rows(0, 2, 1).unwrap(); // A5 -> A4, ref A1 intact
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 4, 1).unwrap(), "11");
}

/// Many formulas sharing one range collapse to a single range vertex; editing a
/// cell inside the range still fans out to every dependent incrementally.
#[test]
fn incremental_shares_one_range_vertex() {
    let mut model = Model::new_empty("m", "en", "UTC", "en").unwrap();
    for row in 1..=10 {
        model.set_user_input(0, row, 1, row.to_string()).unwrap(); // A1..A10
    }
    for row in 1..=50 {
        model
            .set_user_input(0, row, 3, "=SUM(A1:A10)".into())
            .unwrap(); // C1..C50
    }
    model.evaluate();
    model.set_recalc_mode(RecalcMode::Incremental);
    model.evaluate();

    model.set_user_input(0, 5, 1, "100".into()).unwrap(); // A5: 5 -> 100, sum 55 -> 150
    model.evaluate();
    for row in 1..=50 {
        assert_eq!(model.get_formatted_cell_value(0, row, 3).unwrap(), "150");
    }
}

/// A volatile function is recomputed on every incremental edit, matching a full
/// pass. `INDIRECT` reads its target through a string, so no static edge links
/// the target to the reader; only volatile handling makes the reader recompute.
#[test]
fn incremental_recomputes_volatiles() {
    let mut model = Model::new_empty("m", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 3, "5".into()).unwrap(); // C1 = 5
    model
        .set_user_input(0, 1, 1, "=INDIRECT(\"C1\")".into())
        .unwrap(); // A1 reads C1 dynamically
    model.set_user_input(0, 1, 2, "0".into()).unwrap(); // B1 unrelated
    model.evaluate();
    model.set_recalc_mode(RecalcMode::Incremental);
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 1, 1).unwrap(), "5");

    // Edit C1, A1's hidden target. Without volatile handling A1 would stay 5.
    model.set_user_input(0, 1, 3, "99".into()).unwrap();
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 1, 1).unwrap(), "99");
}
