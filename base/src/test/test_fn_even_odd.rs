#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_even_odd_basic() {
    let mut model = new_empty_model();
    model._set("A1", "=EVEN(1)");
    model._set("A2", "=EVEN(3.1)");
    model._set("A3", "=EVEN(-1)");
    model._set("A4", "=EVEN(-3.1)");
    model._set("A5", "=EVEN(0)");

    model._set("B1", "=ODD(2)");
    model._set("B2", "=ODD(5.1)");
    model._set("B3", "=ODD(-2)");
    model._set("B4", "=ODD(-5.1)");
    model._set("B5", "=ODD(0)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"2");
    assert_eq!(model._get_text("A2"), *"4");
    assert_eq!(model._get_text("A3"), *"-2");
    assert_eq!(model._get_text("A4"), *"-4");
    assert_eq!(model._get_text("A5"), *"0");

    assert_eq!(model._get_text("B1"), *"3");
    assert_eq!(model._get_text("B2"), *"7");
    assert_eq!(model._get_text("B3"), *"-3");
    assert_eq!(model._get_text("B4"), *"-7");
    assert_eq!(model._get_text("B5"), *"1");
}

#[test]
fn test_even_odd_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=EVEN()");
    model._set("A2", "=EVEN(1,2)");
    model._set("A3", "=ODD()");
    model._set("A4", "=ODD(1,2)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("A3"), *"#ERROR!");
    assert_eq!(model._get_text("A4"), *"#ERROR!");
}
