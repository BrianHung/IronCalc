#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_sumif_basic() {
    let mut model = new_empty_model();
    // Incorrect number of arguments
    model._set("A1", "=SUMIF()");

    // Without sum_range
    model._set("A2", "=SUMIF(B1:B4,\">2\")");
    // With sum_range
    model._set("A3", "=SUMIF(B1:B4,\">2\",C1:C4)");

    // Data
    model._set("B1", "1");
    model._set("B2", "3");
    model._set("B3", "4");
    model._set("B4", "1");

    model._set("C1", "10");
    model._set("C2", "20");
    model._set("C3", "30");
    model._set("C4", "40");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
    // Sum range omitted -> uses B column
    assert_eq!(model._get_text("A2"), *"7");
    // Sum range provided
    assert_eq!(model._get_text("A3"), *"50");
}

#[test]
fn test_fn_sumif_criteria_operators() {
    let mut model = new_empty_model();

    model._set("A1", "5");
    model._set("A2", "10");
    model._set("A3", "15");
    model._set("A4", "10");
    model._set("A5", "20");

    model._set("B1", "=SUMIF(A1:A5, 10)");
    model._set("B2", "=SUMIF(A1:A5, \">10\")");
    model._set("B3", "=SUMIF(A1:A5, \"<15\")");
    model._set("B4", "=SUMIF(A1:A5, \">=15\")");
    model._set("B5", "=SUMIF(A1:A5, \"<=10\")");
    model._set("B6", "=SUMIF(A1:A5, \"<>10\")");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"20");
    assert_eq!(model._get_text("B2"), *"35");
    assert_eq!(model._get_text("B3"), *"25");
    assert_eq!(model._get_text("B4"), *"35");
    assert_eq!(model._get_text("B5"), *"25");
    assert_eq!(model._get_text("B6"), *"40");
}

#[test]
fn test_fn_sumif_wildcards() {
    let mut model = new_empty_model();

    model._set("A1", "Apple");
    model._set("A2", "Banana");
    model._set("A3", "Apricot");
    model._set("A4", "Cherry");

    model._set("B1", "10");
    model._set("B2", "20");
    model._set("B3", "15");
    model._set("B4", "25");

    model._set("C1", "=SUMIF(A1:A4, \"Ap*\", B1:B4)");
    model._set("C2", "=SUMIF(A1:A4, \"*rry\", B1:B4)");
    model._set("C3", "=SUMIF(A1:A4, \"*a*\", B1:B4)");
    model._set("C4", "=SUMIF(A1:A4, \"<>Apple\", B1:B4)");

    model.evaluate();

    assert_eq!(model._get_text("C1"), *"25");
    assert_eq!(model._get_text("C2"), *"25");
    assert_eq!(model._get_text("C3"), *"45");
    assert_eq!(model._get_text("C4"), *"60");
}

#[test]
fn test_fn_sumif_data_types() {
    let mut model = new_empty_model();

    model._set("A1", "10");
    model._set("A2", "text");
    model._set("A3", "TRUE");
    model._set("A4", "FALSE");
    model._set("A5", "");

    model._set("B1", "1");
    model._set("B2", "2");
    model._set("B3", "3");
    model._set("B4", "4");
    model._set("B5", "5");

    model._set("C1", "=SUMIF(A1:A5, \">5\", B1:B5)");
    model._set("C2", "=SUMIF(A1:A5, \"text\", B1:B5)");
    model._set("C3", "=SUMIF(A1:A5, TRUE, B1:B5)");
    model._set("C4", "=SUMIF(A1:A5, FALSE, B1:B5)");
    model._set("C5", "=SUMIF(A1:A5, \"\", B1:B5)");

    model.evaluate();

    assert_eq!(model._get_text("C1"), *"1");
    assert_eq!(model._get_text("C2"), *"2");
    assert_eq!(model._get_text("C3"), *"3");
    assert_eq!(model._get_text("C4"), *"4");
    assert_eq!(model._get_text("C5"), *"5");
}

#[test]
fn test_fn_sumif_errors() {
    let mut model = new_empty_model();

    model._set("A1", "10");
    model._set("A2", "=1/0");
    model._set("A3", "20");

    model._set("B1", "5");
    model._set("B2", "=NA()");
    model._set("B3", "15");

    model._set("C1", "1");
    model._set("C2", "2");
    model._set("C3", "3");

    model._set("D1", "=SUMIF(A1:A3, \">5\", C1:C3)");
    model._set("D2", "=SUMIF(C1:C3, 2, B1:B3)");

    model.evaluate();

    assert_eq!(model._get_text("D1"), *"4");
    assert_eq!(model._get_text("D2"), *"#N/A");
}

#[test]
fn test_fn_sumif_edge_cases() {
    let mut model = new_empty_model();

    model._set("A1", "0");
    model._set("A2", "-5");
    model._set("A3", "10");

    model._set("B1", "10");
    model._set("B2", "20");
    model._set("B3", "30");

    model._set("C1", "=SUMIF(A1:A3, 0, B1:B3)");
    model._set("C2", "=SUMIF(A1:A3, \"<0\", B1:B3)");
    model._set("C3", "=SUMIF(D1:D3, \"10\", E1:E3)");

    model.evaluate();

    assert_eq!(model._get_text("C1"), *"10");
    assert_eq!(model._get_text("C2"), *"20");
    assert_eq!(model._get_text("C3"), *"0");
}
