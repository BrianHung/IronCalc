#![allow(clippy::unwrap_used)]
use crate::test::util::new_empty_model;

#[test]
fn test_fn_lcm_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=LCM()");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#ERROR!");
}

#[test]
fn test_fn_lcm_basic() {
    let mut model = new_empty_model();
    model._set("A1", "=LCM(25,40)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"200");
}
