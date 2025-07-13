#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_sumproduct_basic() {
    let mut model = new_empty_model();
    model._set("A1", "1");
    model._set("A2", "2");
    model._set("A3", "3");

    model._set("B1", "4");
    model._set("B2", "5");
    model._set("B3", "6");

    model._set("C1", "=SUMPRODUCT(A1:A3,B1:B3)");
    model._set("C2", "=SUMPRODUCT(A1:A3,2)");
    model._set("C3", "=SUMPRODUCT(A1:A3,B1:B2)");

    model.evaluate();

    assert_eq!(model._get_text("C1"), *"32");
    // Scalar second argument broadcast
    assert_eq!(model._get_text("C2"), *"12");
    // Mismatched dimensions produce #VALUE!
    assert_eq!(model._get_text("C3"), *"#VALUE!");
}
