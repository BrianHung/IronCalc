#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn mod_function() {
    let mut model = new_empty_model();
    model._set("A1", "=MOD(9,4)");
    model._set("A2", "=MOD(-3,2)");
    model._set("A3", "=MOD(3,-2)");
    model._set("A4", "=MOD(3,0)");
    model._set("A5", "=MOD(1)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"1");
    assert_eq!(model._get_text("A3"), *"-1");
    assert_eq!(model._get_text("A4"), *"#DIV/0!");
    assert_eq!(model._get_text("A5"), *"#ERROR!");
}

#[test]
fn quotient_function() {
    let mut model = new_empty_model();
    model._set("A1", "=QUOTIENT(5,2)");
    model._set("A2", "=QUOTIENT(5,-2)");
    model._set("A3", "=QUOTIENT(5,0)");
    model._set("A4", "=QUOTIENT(5)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"2");
    assert_eq!(model._get_text("A2"), *"-3");
    assert_eq!(model._get_text("A3"), *"#DIV/0!");
    assert_eq!(model._get_text("A4"), *"#ERROR!");
}
