#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_harmean_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=HARMEAN()");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
}

#[test]
fn test_fn_harmean_minimal() {
    let mut model = new_empty_model();
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("B3", "3");
    model._set("B4", "'2");
    // B5 is empty
    model._set("B6", "true");
    model._set("A1", "=HARMEAN(B1:B6)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"1.636363636");
}

#[test]
fn test_fn_harmean_zero_and_negative() {
    let mut model = new_empty_model();
    model._set("B1", "0");
    model._set("A1", "=HARMEAN(B1)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#NUM!");

    model._set("B1", "-1");
    model._set("A1", "=HARMEAN(B1)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#NUM!");
}

#[test]
fn test_fn_harmean_mathematical_validation() {
    let mut model = new_empty_model();
    // Test with values 1, 2, 4 -> harmonic mean = 3/(1/1 + 1/2 + 1/4) = 3/1.75 ≈ 1.714
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("B3", "4");
    model._set("A1", "=HARMEAN(B1:B3)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"1.714285714");
}
