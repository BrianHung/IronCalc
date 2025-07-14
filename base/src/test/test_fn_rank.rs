#![allow(clippy::unwrap_used)]
use crate::test::util::new_empty_model;

#[test]
fn test_fn_rank() {
    let mut model = new_empty_model();
    model._set("B1", "3");
    model._set("B2", "3");
    model._set("B3", "2");
    model._set("B4", "1");
    model._set("A1", "=RANK(2,B1:B4)");
    model._set("A2", "=RANK.AVG(3,B1:B4)");
    model._set("A3", "=RANK.EQ(3,B1:B4)");
    model._set("A4", "=RANK(3,B1:B4,1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "3");
    assert_eq!(model._get_text("A2"), "1.5");
    assert_eq!(model._get_text("A3"), "1");
    assert_eq!(model._get_text("A4"), "3");
}
