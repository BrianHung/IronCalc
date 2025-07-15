#![allow(clippy::unwrap_used)]
use crate::test::util::new_empty_model;

#[test]
fn test_fn_quartile() {
    let mut model = new_empty_model();
    for i in 1..=8 {
        model._set(&format!("B{i}"), &i.to_string());
    }
    model._set("A1", "=QUARTILE(B1:B8,1)");
    model._set("A2", "=QUARTILE.INC(B1:B8,3)");
    model._set("A3", "=QUARTILE.EXC(B1:B8,1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "2.75");
    assert_eq!(model._get_text("A2"), "6.25");
    assert_eq!(model._get_text("A3"), "2.25");
}
