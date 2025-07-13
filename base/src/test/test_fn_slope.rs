#![allow(clippy::unwrap_used)]
use crate::test::util::new_empty_model;

#[test]
fn test_fn_slope_and_intercept() {
    let mut model = new_empty_model();
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("B3", "3");
    model._set("C1", "2");
    model._set("C2", "4");
    model._set("C3", "6");
    model._set("A1", "=SLOPE(B1:B3,C1:C3)");
    model._set("A2", "=INTERCEPT(B1:B3,C1:C3)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0.5");
    assert_eq!(model._get_text("A2"), *"0");
}

#[test]
fn test_fn_slope_mismatch() {
    let mut model = new_empty_model();
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("B3", "3");
    model._set("C1", "2");
    model._set("C2", "4");
    model._set("A1", "=SLOPE(B1:B3,C1:C2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#N/A");
}
