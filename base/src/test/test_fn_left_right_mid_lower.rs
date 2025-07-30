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

#[test]
fn test_boundary_conditions() {
    let mut model = new_empty_model();
    model._set("A1", "Test");

    model._set("B1", "=LEFT(A1,100)");
    model._set("B2", "=RIGHT(A1,100)");
    model._set("B3", "=MID(A1,1,100)");
    model._set("B4", "=MID(A1,10,5)");
    model._set("B5", "=MID(A1,3,10)");

    model.evaluate();
    assert_eq!(model._get_text("B1"), "Test");
    assert_eq!(model._get_text("B2"), "Test");
    assert_eq!(model._get_text("B3"), "Test");
    assert_eq!(model._get_text("B4"), "");
    assert_eq!(model._get_text("B5"), "st");
}

#[test]
fn test_invalid_parameters() {
    let mut model = new_empty_model();
    model._set("A1", "Hello");

    model._set("B1", "=LEFT(A1,-1)");
    model._set("B2", "=RIGHT(A1,-1)");
    model._set("B3", "=MID(A1,-1,3)");
    model._set("B4", "=MID(A1,0,3)");
    model._set("B5", "=MID(A1,2,-1)");

    model.evaluate();
    assert_eq!(model._get_text("B1"), "#VALUE!");
    assert_eq!(model._get_text("B2"), "#VALUE!");
    assert_eq!(model._get_text("B3"), "#VALUE!");
    assert_eq!(model._get_text("B4"), "#VALUE!");
    assert_eq!(model._get_text("B5"), "#VALUE!");
}

#[test]
fn test_empty_strings() {
    let mut model = new_empty_model();
    model._set("A1", "");
    model._set("A2", "  Space  ");

    model._set("B1", "=LEFT(A1,5)");
    model._set("B2", "=RIGHT(A1,5)");
    model._set("B3", "=MID(A1,1,5)");
    model._set("B4", "=LOWER(A1)");
    model._set("B5", "=LEFT(A2,2)");
    model._set("B6", "=LOWER(A2)");

    model.evaluate();
    assert_eq!(model._get_text("B1"), "");
    assert_eq!(model._get_text("B2"), "");
    assert_eq!(model._get_text("B3"), "");
    assert_eq!(model._get_text("B4"), "");
    assert_eq!(model._get_text("B5"), "  ");
    assert_eq!(model._get_text("B6"), "  space  ");
}
