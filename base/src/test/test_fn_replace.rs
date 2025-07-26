#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn fn_replace_args_number() {
    let mut model = new_empty_model();
    model._set("A1", "abcdef");

    // Too few arguments
    model._set("B1", "=REPLACE(A1, 2, 3)");
    // Too many arguments
    model._set("B2", "=REPLACE(A1, 2, 3, \"X\", \"Y\")");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"#ERROR!");
    assert_eq!(model._get_text("B2"), *"#ERROR!");
}

#[test]
fn fn_replace_basic() {
    let mut model = new_empty_model();
    model._set("A1", "abcdef");

    model._set("B1", "=REPLACE(A1, 2, 3, \"XYZ\")");
    model._set("B2", "=REPLACE(\"12345\", 1, 1, \"abc\")");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"aXYZef");
    assert_eq!(model._get_text("B2"), *"abc2345");
}

#[test]
fn fn_replace_invalid_values() {
    let mut model = new_empty_model();
    model._set("A1", "abcdef");

    // start_num less than 1
    model._set("C1", "=REPLACE(A1, 0, 2, \"x\")");
    // num_chars negative
    model._set("C2", "=REPLACE(A1, 2, -1, \"x\")");

    model.evaluate();

    assert_eq!(model._get_text("C1"), *"#VALUE!");
    assert_eq!(model._get_text("C2"), *"#VALUE!");
}

#[test]
fn fn_replace_start_beyond_length() {
    let mut model = new_empty_model();

    // start_num is greater than the length of old_text → new_text should be appended
    model._set("A1", "=REPLACE(\"abc\", 5, 0, \"X\")");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"abcX");
}
