#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn computation() {
    let mut model = new_empty_model();
    model._set("B1", "0.1");
    model._set("B2", "0.2");
    model._set("A1", "=FVSCHEDULE(100,B1:B2)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), "132");
}
