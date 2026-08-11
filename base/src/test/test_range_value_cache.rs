#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn cache_invalidates_across_passes() {
    let mut model = new_empty_model();
    for row in 1..=20 {
        model._set(&format!("A{row}"), &row.to_string());
    }
    for row in 1..=30 {
        model._set(&format!("C{row}"), "=SUM(A1:A20)");
    }
    model.evaluate();
    for row in 1..=30 {
        assert_eq!(model._get_text(&format!("C{row}")), "210");
    }

    model._set("A5", "105");
    model.evaluate();
    for row in 1..=30 {
        assert_eq!(model._get_text(&format!("C{row}")), "310"); // not the stale 210
    }
}

#[test]
fn range_cache_survives_spill_reorder_restart() {
    let mut model = new_empty_model();
    model._set("A1", "=SEQUENCE(3)*0 + SUM(B11:B20)");
    model._set("B10", "={7;8;9}"); // spills B10:B12, so B11=8, B12=9
    model._set("D1", "=SUM(B11:B20)");
    model.evaluate();
    assert_eq!(model._get_text("D1"), "17"); // not the stale 0
    assert_eq!(model._get_text("A1"), "17");
}

#[test]
fn min_over_open_column_ignores_trailing_blanks() {
    let mut model = new_empty_model();
    model._set("A1", "5");
    model._set("A2", "3");
    model._set("A3", "8");
    model._set("C1", "=MIN(A:A)");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "3");
}

#[test]
fn min_over_skip_header_open_range_stays_correct() {
    let mut model = new_empty_model();
    model._set("A2", "5");
    model._set("A3", "3");
    model._set("C1", "=MIN(A2:A1048576)");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "3");
}

#[test]
fn lcm_over_skip_header_open_range_sees_trailing_blanks() {
    let mut model = new_empty_model();
    model._set("A2", "2");
    model._set("A3", "3");
    model._set("A4", "4");
    model._set("C1", "=LCM(A2:A1048576)");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "0"); // trailing blanks count as 0, lcm(_,0)=0
}

#[test]
fn countblank_over_open_column_counts_trailing_blanks() {
    let mut model = new_empty_model();
    model._set("A1", "1");
    model._set("A2", "2");
    model._set("C1", "=COUNTBLANK(A:A)");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "1048574"); // whole column minus the two filled
}
