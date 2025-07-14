#![allow(clippy::unwrap_used)]
use crate::test::util::new_empty_model;

#[test]
fn test_fn_var_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=VAR.S()");
    model._set("A2", "=VAR.P()");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
}

#[test]
fn test_fn_var_minimal() {
    let mut model = new_empty_model();
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("B3", "3");
    model._set("B4", "4");
    model._set("B5", "5");
    model._set("A1", "=VAR.S(B1:B5)");
    model._set("A2", "=VAR.P(B1:B5)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"2.5");
    assert_eq!(model._get_text("A2"), *"2");
}
