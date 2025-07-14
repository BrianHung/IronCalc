#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_slope_intercept_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=SLOPE()");
    model._set("A2", "=INTERCEPT(B1:B3)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
}

#[test]
fn test_fn_slope_intercept_minimal() {
    let mut model = new_empty_model();
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("B3", "3");
    model._set("C1", "1");
    model._set("C2", "2");
    model._set("C3", "5");
    model._set("A1", "=SLOPE(B1:B3,C1:C3)");
    model._set("A2", "=INTERCEPT(B1:B3,C1:C3)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0.461538462");
    assert_eq!(model._get_text("A2"), *"0.769230769");
}
