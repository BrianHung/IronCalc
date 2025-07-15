#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_xmatch_basic() {
    let mut model = new_empty_model();
    model._set("A1", "10");
    model._set("A2", "20");
    model._set("A3", "30");
    model._set("B1", "=XMATCH(20, A1:A3)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"2");
}

#[test]
fn test_fn_xmatch_wildcard() {
    let mut model = new_empty_model();
    model._set("A1", "apple");
    model._set("A2", "banana");
    model._set("A3", "apricot");
    model._set("B1", "=XMATCH(\"ap*\", A1:A3, 2)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"1");
}
