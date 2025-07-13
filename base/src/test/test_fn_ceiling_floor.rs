#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn simple_cases() {
    let mut model = new_empty_model();
    model._set("A1", "=CEILING(4.3,2)");
    model._set("A2", "=CEILING(-4.3,-2)");
    model._set("A3", "=CEILING(-4.3,2)");
    model._set("B1", "=FLOOR(4.3,2)");
    model._set("B2", "=FLOOR(-4.3,-2)");
    model._set("B3", "=FLOOR(4.3,-2)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"6");
    assert_eq!(model._get_text("A2"), *"-4");
    assert_eq!(model._get_text("A3"), *"#NUM!");
    assert_eq!(model._get_text("B1"), *"4");
    assert_eq!(model._get_text("B2"), *"-6");
    assert_eq!(model._get_text("B3"), *"#NUM!");
}

#[test]
fn wrong_number_of_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=CEILING(1)");
    model._set("A2", "=CEILING(1,2,3)");
    model._set("B1", "=FLOOR(1)");
    model._set("B2", "=FLOOR(1,2,3)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("B1"), *"#ERROR!");
    assert_eq!(model._get_text("B2"), *"#ERROR!");
}
