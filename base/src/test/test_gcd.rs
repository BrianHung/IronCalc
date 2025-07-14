#![allow(clippy::unwrap_used)]
use crate::test::util::new_empty_model;

#[test]
fn test_fn_gcd_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=GCD()");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#ERROR!");
}

#[test]
fn test_fn_gcd_basic() {
    let mut model = new_empty_model();
    model._set("A1", "=GCD(60,36)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"12");
}
