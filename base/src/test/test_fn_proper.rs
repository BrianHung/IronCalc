#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn fn_proper_args_number() {
    let mut model = new_empty_model();

    // No arguments
    model._set("A1", "=PROPER()");
    // Too many arguments
    model._set("A2", "=PROPER(\"text\", \"extra\")");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
}

#[test]
fn fn_proper_basic() {
    let mut model = new_empty_model();
    model._set("A1", "one TWO");

    model._set("B1", "=PROPER(A1)");
    model._set("B2", "=PROPER(\"mcdonald\")");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"One Two");
    assert_eq!(model._get_text("B2"), *"Mcdonald");
}

#[test]
fn fn_proper_punctuation() {
    let mut model = new_empty_model();

    model._set("A1", "=PROPER(\"o'reilly\")");
    model._set("A2", "=PROPER(\"smith-jones\")");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"O'Reilly");
    assert_eq!(model._get_text("A2"), *"Smith-Jones");
}
