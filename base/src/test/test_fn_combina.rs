#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_combina_results() {
    let mut model = new_empty_model();
    model._set("A1", "=COMBINA(10,3)");
    model._set("A2", "=COMBINA(3.7,2.2)");
    model._set("A3", "=COMBINA(4,0)");
    model._set("A4", "=COMBINA(0,3)");
    model._set("A5", "=COMBINA(0,0)");
    model._set("A6", "=COMBINA(4,3)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"220");
    assert_eq!(model._get_text("A2"), *"6");
    assert_eq!(model._get_text("A3"), *"1");
    assert_eq!(model._get_text("A4"), *"0");
    assert_eq!(model._get_text("A5"), *"1");
    assert_eq!(model._get_text("A6"), *"20");
}

#[test]
fn test_fn_combina_errors() {
    let mut model = new_empty_model();
    model._set("B1", "=COMBINA(-1,2)");
    model._set("B2", "=COMBINA(3,-2)");
    model._set("B3", "=COMBINA(1)");
    model._set("B4", "=COMBINA(1,2,3)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"#NUM!");
    assert_eq!(model._get_text("B2"), *"#NUM!");
    assert_eq!(model._get_text("B3"), *"#ERROR!");
    assert_eq!(model._get_text("B4"), *"#ERROR!");
}
