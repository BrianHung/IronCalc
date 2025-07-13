#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_sumif_basic() {
    let mut model = new_empty_model();
    // Incorrect number of arguments
    model._set("A1", "=SUMIF()");

    // Without sum_range
    model._set("A2", "=SUMIF(B1:B4,\">2\")");
    // With sum_range
    model._set("A3", "=SUMIF(B1:B4,\">2\",C1:C4)");

    // Data
    model._set("B1", "1");
    model._set("B2", "3");
    model._set("B3", "4");
    model._set("B4", "1");

    model._set("C1", "10");
    model._set("C2", "20");
    model._set("C3", "30");
    model._set("C4", "40");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
    // Sum range omitted -> uses B column
    assert_eq!(model._get_text("A2"), *"7");
    // Sum range provided
    assert_eq!(model._get_text("A3"), *"50");
}
