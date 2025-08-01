#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=SIGN()");
    model._set("A2", "=SIGN(1,2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
}

#[test]
fn values() {
    let mut model = new_empty_model();
    model._set("A1", "=SIGN(5)");
    model._set("A2", "=SIGN(-3)");
    model._set("A3", "=SIGN(0)");
    model._set("A4", "=SIGN(0.000001)");
    model._set("A5", "=SIGN(-0.000001)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"-1");
    assert_eq!(model._get_text("A3"), *"0");
    assert_eq!(model._get_text("A4"), *"1");
    assert_eq!(model._get_text("A5"), *"-1");
}

#[test]
fn extreme_values() {
    let mut model = new_empty_model();
    model._set("A1", "=SIGN(1E308)"); // Very large positive
    model._set("A2", "=SIGN(-1E308)"); // Very large negative
    model._set("A3", "=SIGN(1E-308)"); // Very small positive
    model._set("A4", "=SIGN(-1E-308)"); // Very small negative
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"-1");
    assert_eq!(model._get_text("A3"), *"1");
    assert_eq!(model._get_text("A4"), *"-1");
}
