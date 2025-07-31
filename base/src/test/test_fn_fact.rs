#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn fact_comprehensive() {
    let mut model = new_empty_model();

    model._set("A1", "=FACT(0)");
    model._set("A2", "=FACT(3)");
    model._set("A3", "=FACT(5)");
    model._set("A4", "=FACT(170)");
    model._set("A5", "=FACT(171)");
    model._set("A6", "=FACT(3.7)");
    model._set("A7", "=FACT(-1)");
    model._set("A8", "=FACT()");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"6");
    assert_eq!(model._get_text("A3"), *"120");
    assert_eq!(model._get_text("A4"), *"7.25742E+306");
    assert_eq!(model._get_text("A5"), *"#NUM!");
    assert_eq!(model._get_text("A6"), *"6");
    assert_eq!(model._get_text("A7"), *"#NUM!");
    assert_eq!(model._get_text("A8"), *"#ERROR!");
}

#[test]
fn fact_overflow() {
    let mut model = new_empty_model();

    model._set("A1", "=FACT(170)");
    model._set("A2", "=FACT(171)");
    model._set("A3", "=FACT(200)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"7.25742E+306");
    assert_eq!(model._get_text("A2"), *"#NUM!");
    assert_eq!(model._get_text("A3"), *"#NUM!");
}
