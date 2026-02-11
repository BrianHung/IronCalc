#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_row_no_arg_and_reference() {
    let mut model = new_empty_model();
    model._set("B5", "=ROW()");
    model._set("B6", "=ROW(A7)");
    model._set("B7", "=ROW(B5:B10)");

    model.evaluate();

    assert_eq!(model._get_text("B5"), *"5");
    assert_eq!(model._get_text("B6"), *"7");
    // Ranges return first row
    assert_eq!(model._get_text("B7"), *"5");
}

#[test]
fn test_rows_function() {
    let mut model = new_empty_model();
    model._set("C1", "=ROWS(A1:A4)");
    model._set("C2", "=ROWS(B5:B5)");

    model.evaluate();

    assert_eq!(model._get_text("C1"), *"4");
    assert_eq!(model._get_text("C2"), *"1");
}

#[test]
fn test_column_no_arg_and_reference() {
    let mut model = new_empty_model();
    model._set("D3", "=COLUMN()");
    model._set("D4", "=COLUMN(C5)");
    model._set("D5", "=COLUMN(D3:F3)");

    model.evaluate();

    assert_eq!(model._get_text("D3"), *"4");
    assert_eq!(model._get_text("D4"), *"3");
    // Ranges return first column
    assert_eq!(model._get_text("D5"), *"4");
}

#[test]
fn test_columns_function() {
    let mut model = new_empty_model();
    model._set("E1", "=COLUMNS(A1:C1)");
    model._set("E2", "=COLUMNS(D4:D8)");

    model.evaluate();

    assert_eq!(model._get_text("E1"), *"3");
    assert_eq!(model._get_text("E2"), *"1");
}
