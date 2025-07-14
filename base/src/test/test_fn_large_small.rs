#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_large_small_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=LARGE()");
    model._set("A2", "=LARGE(B1:B5)");
    model._set("A3", "=SMALL()");
    model._set("A4", "=SMALL(B1:B5)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("A3"), *"#ERROR!");
    assert_eq!(model._get_text("A4"), *"#ERROR!");
}

#[test]
fn test_fn_large_small_basic() {
    let mut model = new_empty_model();
    model._set("B1", "1");
    model._set("B2", "3");
    model._set("B3", "5");
    model._set("B4", "7");
    model._set("B5", "9");
    model._set("A1", "=LARGE(B1:B5,2)");
    model._set("A2", "=SMALL(B1:B5,3)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"7");
    assert_eq!(model._get_text("A2"), *"5");
}
