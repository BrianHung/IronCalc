#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn permutationa_comprehensive() {
    let mut model = new_empty_model();

    model._set("A1", "=PERMUTATIONA(0,0)");
    model._set("A2", "=PERMUTATIONA(0,3)");
    model._set("A3", "=PERMUTATIONA(5,0)");
    model._set("A4", "=PERMUTATIONA(10,3)");
    model._set("A5", "=PERMUTATIONA(5.9,2.9)");
    model._set("A6", "=PERMUTATIONA(-1,2)");
    model._set("A7", "=PERMUTATIONA()");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"0");
    assert_eq!(model._get_text("A3"), *"1");
    assert_eq!(model._get_text("A4"), *"1000");
    assert_eq!(model._get_text("A5"), *"25");
    assert_eq!(model._get_text("A6"), *"#NUM!");
    assert_eq!(model._get_text("A7"), *"#ERROR!");
}

#[test]
fn permutationa_power_equivalence() {
    let mut model = new_empty_model();

    model._set("A1", "=PERMUTATIONA(4,3)");
    model._set("A2", "=POWER(4,3)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), model._get_text("A2"));
}
