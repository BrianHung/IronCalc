#![allow(clippy::unwrap_used)]

//! Range composition reuses a one-row-shorter range plus its last row when that
//! shorter range is itself referenced, so overlapping aggregates cost O(n)
//! instead of O(n^2). These tests check the result matches a direct scan for the
//! running-aggregate pattern that triggers it, including the per-function quirks
//! (Min/Max ignore non-numbers and empty ranges are 0; Count ignores errors).

use crate::test::util::new_empty_model;

// A multi-column running SUM composes across rows (each row reduced then folded
// into the running total), so this exercises the multi-column path where the
// summation is re-associated. Integer inputs make the running totals exact, so
// the composed result must still equal the running column-major total.
#[test]
fn sum_multi_column_running() {
    let mut model = new_empty_model();
    // A/B hold two columns of values; D_k = SUM(A1:B_k) is a growing 2-wide range.
    let rows = [(1, 2), (3, 4), (5, 6), (7, 8)];
    for (i, (a, b)) in rows.iter().enumerate() {
        let row = i as i32 + 1;
        model._set(&format!("A{row}"), &a.to_string());
        model._set(&format!("B{row}"), &b.to_string());
        model._set(&format!("D{row}"), &format!("=SUM(A1:B{row})"));
    }

    model.evaluate();

    let mut running = 0;
    for (i, (a, b)) in rows.iter().enumerate() {
        let row = i as i32 + 1;
        running += a + b;
        assert_eq!(model._get_text(&format!("D{row}")), running.to_string());
    }
}

// Running MIN/MAX over an increasing then decreasing column.
#[test]
fn min_max_running() {
    let mut model = new_empty_model();
    let values = [5, 3, 8, 1, 9, 2];
    for (i, v) in values.iter().enumerate() {
        let row = i as i32 + 1;
        model._set(&format!("A{row}"), &v.to_string());
        model._set(&format!("B{row}"), &format!("=MIN(A1:A{row})"));
        model._set(&format!("C{row}"), &format!("=MAX(A1:A{row})"));
    }

    model.evaluate();

    let mut running_min = i32::MAX;
    let mut running_max = i32::MIN;
    for (i, v) in values.iter().enumerate() {
        let row = i as i32 + 1;
        running_min = running_min.min(*v);
        running_max = running_max.max(*v);
        assert_eq!(model._get_text(&format!("B{row}")), running_min.to_string());
        assert_eq!(model._get_text(&format!("C{row}")), running_max.to_string());
    }
}

// Min/Max ignore text and blanks and an all-empty range reduces to 0.
#[test]
fn min_max_ignore_non_numbers() {
    let mut model = new_empty_model();
    model._set("A2", "text");
    model._set("A3", "7");
    for row in 1..=3 {
        model._set(&format!("B{row}"), &format!("=MIN(A1:A{row})"));
        model._set(&format!("C{row}"), &format!("=MAX(A1:A{row})"));
    }

    model.evaluate();

    // A1 blank, A2 text: no numbers yet, so MIN/MAX are 0.
    assert_eq!(model._get_text("B2"), *"0");
    assert_eq!(model._get_text("C2"), *"0");
    assert_eq!(model._get_text("B3"), *"7");
    assert_eq!(model._get_text("C3"), *"7");
}

// Min/Max propagate the first error in row-major order.
#[test]
fn min_propagates_first_error() {
    let mut model = new_empty_model();
    model._set("A1", "1");
    model._set("A2", "=1/0");
    for row in 1..=2 {
        model._set(&format!("B{row}"), &format!("=MIN(A1:A{row})"));
    }

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"1");
    assert_eq!(model._get_text("B2"), *"#DIV/0!");
}

// Running COUNT of numeric cells, skipping text, blanks, and errors.
#[test]
fn count_running_ignores_non_numbers_and_errors() {
    let mut model = new_empty_model();
    model._set("A1", "10");
    model._set("A2", "text");
    model._set("A3", "=1/0");
    model._set("A4", "20");
    for row in 1..=4 {
        model._set(&format!("B{row}"), &format!("=COUNT(A1:A{row})"));
    }

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"1");
    assert_eq!(model._get_text("B2"), *"1");
    assert_eq!(model._get_text("B3"), *"1");
    assert_eq!(model._get_text("B4"), *"2");
}
