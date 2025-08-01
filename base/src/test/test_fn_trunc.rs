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
    model._set("A5", "=TRUNC(3.141593,0)"); // Zero digits - truncate to integer
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"4");
    assert_eq!(model._get_text("A2"), *"-3");
    assert_eq!(model._get_text("A3"), *"3.14");
    assert_eq!(model._get_text("A4"), *"990");
    assert_eq!(model._get_text("A5"), *"3");
}

#[test]
fn edge_cases() {
    let mut model = new_empty_model();
    model._set("A1", "=TRUNC(0)"); // Zero input
    model._set("A2", "=TRUNC(-0.999)"); // Negative close to zero
    model._set("A3", "=TRUNC(123.456,-2)"); // Truncate to hundreds
    model._set("A4", "=TRUNC(12345.6789,3)"); // More decimal places than input
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0");
    assert_eq!(model._get_text("A2"), *"0");
    assert_eq!(model._get_text("A3"), *"100");
    assert_eq!(model._get_text("A4"), *"12345.678");
}
