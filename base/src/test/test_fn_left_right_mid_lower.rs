#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_left() {
    let mut model = new_empty_model();
    model._set("A1", "Hello");
    model._set("B1", "=LEFT(A1,3)");
    model._set("B2", "=LEFT(A1)");
    model._set("B3", "=LEFT(A1,0)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"Hel");
    assert_eq!(model._get_text("B2"), *"H");
    assert_eq!(model._get_text("B3"), "");
}

#[test]
fn test_right() {
    let mut model = new_empty_model();
    model._set("A1", "Hello");
    model._set("B1", "=RIGHT(A1,2)");
    model._set("B2", "=RIGHT(A1)");
    model._set("B3", "=RIGHT(A1,0)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"lo");
    assert_eq!(model._get_text("B2"), *"o");
    assert_eq!(model._get_text("B3"), "");
}

#[test]
fn test_mid() {
    let mut model = new_empty_model();
    model._set("A1", "Hello");
    model._set("B1", "=MID(A1,2,3)");
    model._set("B2", "=MID(A1,1,2)");
    model._set("B3", "=MID(A1,1,0)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"ell");
    assert_eq!(model._get_text("B2"), *"He");
    assert_eq!(model._get_text("B3"), "");
}

#[test]
fn test_lower() {
    let mut model = new_empty_model();
    model._set("A1", "Hello WORLD");
    model._set("B1", "=LOWER(A1)");
    model._set("B2", "=LOWER(\"TEST\")");
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"hello world");
    assert_eq!(model._get_text("B2"), *"test");
}
