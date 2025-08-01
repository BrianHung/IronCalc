#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn ceiling_math_examples() {
    let mut model = new_empty_model();
    model._set("A1", "=CEILING.MATH(24.3,5)");
    model._set("A2", "=CEILING.MATH(6.7)");
    model._set("A3", "=CEILING.MATH(-8.1,2)");
    model._set("A4", "=CEILING.MATH(-5.5,2,-1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"25");
    assert_eq!(model._get_text("A2"), *"7");
    assert_eq!(model._get_text("A3"), *"-8");
    assert_eq!(model._get_text("A4"), *"-6");
}

#[test]
fn ceiling_precise_and_iso() {
    let mut model = new_empty_model();
    model._set("A1", "=CEILING.PRECISE(4.3)");
    model._set("A2", "=CEILING.PRECISE(-4.3)");
    model._set("A3", "=ISO.CEILING(4.3,2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"5");
    assert_eq!(model._get_text("A2"), *"-4");
    assert_eq!(model._get_text("A3"), *"6");
}

#[test]
fn floor_math_and_precise() {
    let mut model = new_empty_model();
    model._set("A1", "=FLOOR.MATH(24.3,5)");
    model._set("A2", "=FLOOR.MATH(-8.1,2)");
    model._set("A3", "=FLOOR.MATH(-5.5,2,-1)");
    model._set("A4", "=FLOOR.PRECISE(-4.3)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"20");
    assert_eq!(model._get_text("A2"), *"-10");
    assert_eq!(model._get_text("A3"), *"-4");
    assert_eq!(model._get_text("A4"), *"-4");
}
