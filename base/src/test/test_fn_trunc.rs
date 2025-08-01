#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=TRUNC()");
    model._set("A2", "=TRUNC(1,2,3)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
}

#[test]
fn values() {
    let mut model = new_empty_model();
    model._set("A1", "=TRUNC(4.9)");
    model._set("A2", "=TRUNC(-3.5)");
    model._set("A3", "=TRUNC(3.141593,2)");
    model._set("A4", "=TRUNC(999.99,-1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"4");
    assert_eq!(model._get_text("A2"), *"-3");
    assert_eq!(model._get_text("A3"), *"3.14");
    assert_eq!(model._get_text("A4"), *"990");
}
