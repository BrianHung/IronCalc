#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_norm_functions() {
    let mut model = new_empty_model();
    model._set("A1", "=NORM.DIST(5,3,2,TRUE)");
    model._set("A2", "=NORM.DIST(5,3,2,FALSE)");
    model._set("A3", "=NORM.INV(0.5,3,2)");
    model._set("A4", "=NORM.S.DIST(1,TRUE)");
    model._set("A5", "=NORM.S.DIST(1,FALSE)");
    model._set("A6", "=NORM.S.INV(0.841344746)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0.841344746");
    assert_eq!(model._get_text("A2"), *"0.120985362");
    assert_eq!(model._get_text("A3"), *"3");
    assert_eq!(model._get_text("A4"), *"0.841344746");
    assert_eq!(model._get_text("A5"), *"0.241970725");
    assert_eq!(model._get_text("A6"), *"1");
}
