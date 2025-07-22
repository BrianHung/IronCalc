#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_sumproduct_basic() {
    let mut model = new_empty_model();
    model._set("A1", "1");
    model._set("A2", "2");
    model._set("A3", "3");

    model._set("B1", "4");
    model._set("B2", "5");
    model._set("B3", "6");

    model._set("C1", "=SUMPRODUCT(A1:A3,B1:B3)");
    model._set("C2", "=SUMPRODUCT(A1:A3,2)");
    model._set("C3", "=SUMPRODUCT(A1:A3,B1:B2)");

    model.evaluate();

    assert_eq!(model._get_text("C1"), *"32");
    assert_eq!(model._get_text("C2"), *"12");
    assert_eq!(model._get_text("C3"), *"#VALUE!");
}

#[test]
fn test_fn_sumproduct_scalars() {
    let mut model = new_empty_model();

    model._set("A1", "=SUMPRODUCT()");
    model._set("A2", "=SUMPRODUCT(5)");
    model._set("A3", "=SUMPRODUCT(2, 3)");
    model._set("A4", "=SUMPRODUCT(2, 3, 4)");
    model._set("A5", "=SUMPRODUCT(0, 5)");
    model._set("A6", "=SUMPRODUCT(-2, 3)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"5");
    assert_eq!(model._get_text("A3"), *"6");
    assert_eq!(model._get_text("A4"), *"24");
    assert_eq!(model._get_text("A5"), *"0");
    assert_eq!(model._get_text("A6"), *"-6");
}

#[test]
fn test_fn_sumproduct_data_types() {
    let mut model = new_empty_model();

    model._set("A1", "1");
    model._set("A2", "TRUE");
    model._set("A3", "5");
    model._set("A4", "hello");
    model._set("A5", "2.5");

    model._set("B1", "2");
    model._set("B2", "FALSE");
    model._set("B3", "");
    model._set("B4", "4");
    model._set("B5", "1");

    model._set("C1", "=SUMPRODUCT(A1:A5, B1:B5)");

    model._set("D1", "2");
    model._set("D2", "3");
    model._set("D3", "4");
    model._set("E1", "=SUMPRODUCT(D1:D3)");

    model.evaluate();

    assert_eq!(model._get_text("C1"), *"4.5");
    assert_eq!(model._get_text("E1"), *"9");
}

#[test]
fn test_fn_sumproduct_arrays() {
    let mut model = new_empty_model();

    model._set("A1", "1");
    model._set("A2", "2");
    model._set("B1", "3");
    model._set("B2", "4");
    model._set("C1", "5");
    model._set("C2", "6");

    model._set("D1", "=SUMPRODUCT(A1:A2, B1:B2, C1:C2)");
    model._set("D2", "=SUMPRODUCT(A1:A2, 2, B1:B2)");

    model._set("E1", "1");
    model._set("E2", "2");
    model._set("F1", "3");
    model._set("F2", "4");
    model._set("G1", "5");
    model._set("G2", "6");
    model._set("H1", "7");
    model._set("H2", "8");

    model._set("D3", "=SUMPRODUCT(E1:F2, G1:H2)");

    model.evaluate();

    assert_eq!(model._get_text("D1"), *"63");
    assert_eq!(model._get_text("D2"), *"22");
    assert_eq!(model._get_text("D3"), *"70");
}

#[test]
fn test_fn_sumproduct_errors() {
    let mut model = new_empty_model();

    model._set("A1", "1");
    model._set("A2", "=1/0");
    model._set("A3", "3");

    model._set("B1", "4");
    model._set("B2", "5");
    model._set("B3", "6");

    model._set("C1", "=SUMPRODUCT(A1:A3, B1:B3)");

    model._set("D1", "=1/0");
    model._set("C2", "=SUMPRODUCT(A1, D1)");

    model.evaluate();

    assert_eq!(model._get_text("C1"), *"#DIV/0!");
    assert_eq!(model._get_text("C2"), *"#DIV/0!");
}

#[test]
fn test_fn_sumproduct_edge_cases() {
    let mut model = new_empty_model();

    model._set("A1", "0");
    model._set("A2", "-1");
    model._set("B1", "5");
    model._set("B2", "3");

    model._set("C1", "1");
    model._set("C2", "2");
    model._set("D1", "3");
    model._set("D2", "4");
    model._set("D3", "5");

    model._set("E1", "=SUMPRODUCT(A1:A2, B1:B2)");
    model._set("E2", "=SUMPRODUCT(C1:C2, D1:D3)");

    model.evaluate();

    assert_eq!(model._get_text("E1"), *"-3");
    assert_eq!(model._get_text("E2"), *"#VALUE!");
}
