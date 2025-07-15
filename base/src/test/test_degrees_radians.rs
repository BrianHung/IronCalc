#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn degrees_and_radians_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=DEGREES()");
    model._set("A2", "=DEGREES(PI())");
    model._set("A3", "=DEGREES(1,2)");

    model._set("B1", "=RADIANS()");
    model._set("B2", "=RADIANS(180)");
    model._set("B3", "=RADIANS(1,2)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"180");
    assert_eq!(model._get_text("A3"), *"#ERROR!");

    assert_eq!(model._get_text("B1"), *"#ERROR!");
    assert_eq!(model._get_text("B2"), *"3.141592654");
    assert_eq!(model._get_text("B3"), *"#ERROR!");
}
