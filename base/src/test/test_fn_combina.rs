#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn combina_comprehensive() {
    let mut model = new_empty_model();

    model._set("A1", "=COMBINA(0,0)");
    model._set("A2", "=COMBINA(0,3)");
    model._set("A3", "=COMBINA(4,3)");
    model._set("A4", "=COMBINA(3,1)");
    model._set("A5", "=COMBINA(4.9,2.1)");
    model._set("A6", "=COMBINA(-1,2)");
    model._set("A7", "=COMBINA()");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"0");
    assert_eq!(model._get_text("A3"), *"20");
    assert_eq!(model._get_text("A4"), *"3");
    assert_eq!(model._get_text("A5"), *"10");
    assert_eq!(model._get_text("A6"), *"#NUM!");
    assert_eq!(model._get_text("A7"), *"#ERROR!");
}

#[test]
fn combina_mathematical_identity() {
    let mut model = new_empty_model();

    model._set("A1", "=COMBINA(4,3)");
    model._set("A2", "=COMBIN(6,3)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), model._get_text("A2"));
}
