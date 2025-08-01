#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=EXP()");
    model._set("A2", "=EXP(1,2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
}

#[test]
fn values() {
    let mut model = new_empty_model();
    model._set("A1", "=EXP(0)");
    model._set("A2", "=EXP(1)");
    model._set("A3", "=EXP(2)");
    model._set("A4", "=EXP(-1)");
    model._set("A5", "=EXP(-2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"2.718281828");
    assert_eq!(model._get_text("A3"), *"7.389056099");
    assert_eq!(model._get_text("A4"), *"0.367879441");
    assert_eq!(model._get_text("A5"), *"0.135335283");
}

#[test]
fn overflow() {
    let mut model = new_empty_model();
    model._set("A1", "=EXP(1000)"); // Should trigger overflow
    model._set("A2", "=EXP(-1000)"); // Very small, should not overflow
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#DIV/0!");
    assert_eq!(model._get_text("A2"), *"0");
}

#[test]
fn edge_cases() {
    let mut model = new_empty_model();
    model._set("A1", "=EXP(709)"); // Near overflow boundary
    model._set("A2", "=EXP(-745)"); // Very small result
    model._set("A3", "=EXP(0.5)"); // Fractional input
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"8.21841E+307");
    assert_eq!(model._get_text("A2"), *"infE-324");
    assert_eq!(model._get_text("A3"), *"1.648721271");
}
