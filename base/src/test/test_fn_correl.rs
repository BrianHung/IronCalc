#![allow(clippy::unwrap_used)]
use crate::test::util::new_empty_model;

#[test]
fn test_fn_correl_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=CORREL(B1:B2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#ERROR!");
}

#[test]
fn test_fn_correl_minimal() {
    let mut model = new_empty_model();
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("B3", "3");
    model._set("B4", "4");
    model._set("B5", "5");
    model._set("C1", "2");
    model._set("C2", "4");
    model._set("C3", "6");
    model._set("C4", "8");
    model._set("C5", "10");
    model._set("A1", "=CORREL(B1:B5, C1:C5)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"1");
}
