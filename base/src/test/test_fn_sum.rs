#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_sum_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=SUM()");
    model._set("A2", "=SUM(1, 2, 3)");
    model._set("A3", "=SUM(1, )");
    model._set("A4", "=SUM(1,   , 3)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"6");
    assert_eq!(model._get_text("A3"), *"1");
    assert_eq!(model._get_text("A4"), *"4");
}

#[test]
fn arrays() {
    let mut model = new_empty_model();
    model._set("A1", "=SUM({1, 2, 3})");
    model._set("A2", "=SUM({1; 2; 3})");
    model._set("A3", "=SUM({1, 2; 3, 4})");
    model._set("A4", "=SUM({1, 2; 3, 4; 5, 6})");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"6");
    assert_eq!(model._get_text("A2"), *"6");
    assert_eq!(model._get_text("A3"), *"10");
    assert_eq!(model._get_text("A4"), *"21");
}

#[test]
fn test_fn_sum_text_converted_to_number() {
    let mut model = new_empty_model();

    model._set("A1", r#"=SUM("1")"#);
    model._set("A2", r#"=SUM("1e2")"#);

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"100");
}

#[test]
fn test_fn_sum_invalid_text() {
    let mut model = new_empty_model();

    model._set("A1", r#"=SUM("a")"#);

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#VALUE!");
}

#[test]
fn test_fn_sum_text_in_range_not_converted() {
    let mut model = new_empty_model();

    model._set("A1", r#"=SUM(B1:D1)"#);
    model._set("B1", r#"="100""#);

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"0");
}

#[test]
fn test_fn_sum_text_in_reference_not_converted() {
    let mut model = new_empty_model();

    model._set("A1", r#"=SUM(B1)"#);
    model._set("B1", r#"="100""#);

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"0");
}

#[test]
fn test_fn_sum_text_in_indirect_reference_not_converted() {
    let mut model = new_empty_model();

    model._set("A1", r#"=SUM(INDIRECT("B1"))"#);
    model._set("B1", r#"="100""#);

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"0");
}

#[test]
fn test_fn_sum_text_in_indirect_reference() {
    let mut model = new_empty_model();

    model._set("A1", r#"=SUM(INDIRECT("B1"))"#);
    model._set("B1", r#"100"#);

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"100");
}

#[test]
fn test_fn_sum_invalid_text_in_range() {
    let mut model = new_empty_model();

    model._set("A1", r#"=SUM(B1:D1)"#);
    model._set("B1", "a");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"0");
}

#[test]
fn test_fn_sum_invalid_text_in_reference() {
    let mut model = new_empty_model();

    model._set("A1", r#"=SUM(B1)"#);
    model._set("B1", r#"a"#);

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"0");
}

#[test]
fn test_fn_sum_boolean_values_converted() {
    let mut model = new_empty_model();

    model._set("A1", r#"=SUM(TRUE)"#);
    model._set("A2", r#"=SUM(FALSE)"#);

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"0");
}

// Running totals: every prefix SUM(A1:Ak) is a referenced range, so range
// composition reduces the family by reusing the range one row shorter.
#[test]
fn test_fn_sum_range_composition_running_totals() {
    let mut model = new_empty_model();
    for row in 1..=20 {
        model._set(&format!("A{row}"), &row.to_string());
        model._set(&format!("B{row}"), &format!("=SUM(A1:A{row})"));
    }

    model.evaluate();

    for row in 1..=20 {
        let expected = row * (row + 1) / 2;
        assert_eq!(model._get_text(&format!("B{row}")), expected.to_string());
    }
}

// The first error in row-major order is propagated, exactly as a direct scan.
#[test]
fn test_fn_sum_range_composition_propagates_first_error() {
    let mut model = new_empty_model();
    model._set("A1", "1");
    model._set("A2", "=1/0");
    model._set("A3", "=NA()");
    for row in 1..=3 {
        model._set(&format!("B{row}"), &format!("=SUM(A1:A{row})"));
    }

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"1");
    assert_eq!(model._get_text("B2"), *"#DIV/0!");
    assert_eq!(model._get_text("B3"), *"#DIV/0!");
}

// Text and blank cells inside the composed range are ignored, like a direct scan.
#[test]
fn test_fn_sum_range_composition_ignores_text_and_blanks() {
    let mut model = new_empty_model();
    model._set("A1", "1");
    model._set("A2", "text");
    model._set("A4", "4");
    for row in 1..=4 {
        model._set(&format!("B{row}"), &format!("=SUM(A1:A{row})"));
    }

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"1");
    assert_eq!(model._get_text("B2"), *"1");
    assert_eq!(model._get_text("B3"), *"1");
    assert_eq!(model._get_text("B4"), *"5");
}

// A full-column reference is clamped to the sheet before reduction.
#[test]
fn test_fn_sum_range_composition_full_column() {
    let mut model = new_empty_model();
    model._set("A1", "10");
    model._set("A2", "20");
    model._set("A3", "30");
    model._set("B1", "=SUM(A:A)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"60");
}
