#![allow(clippy::unwrap_used)]
use crate::test::util::new_empty_model;

#[test]
fn test_fn_ceiling() {
    let mut model = new_empty_model();
    model._set("A1", "=CEILING(4.3,2)");
    model._set("A2", "=CEILING(-4.3,2)");
    model._set("A3", "=CEILING(4.3,-2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"6");
    assert_eq!(model._get_text("A2"), *"-4");
    assert_eq!(model._get_text("A3"), *"#NUM!");
}

#[test]
fn test_fn_floor() {
    let mut model = new_empty_model();
    model._set("B1", "=FLOOR(4.3,2)");
    model._set("B2", "=FLOOR(-4.3,2)");
    model._set("B3", "=FLOOR(4.3,-2)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"4");
    assert_eq!(model._get_text("B2"), *"-6");
    assert_eq!(model._get_text("B3"), *"#NUM!");
}
