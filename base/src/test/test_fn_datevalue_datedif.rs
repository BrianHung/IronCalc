#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_datevalue_basic() {
    let mut model = new_empty_model();
    model._set("A1", "=DATEVALUE(\"2/1/2023\")");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"02/01/2023");
}

#[test]
fn test_datedif_basic() {
    let mut model = new_empty_model();
    model._set("A1", "=DATEDIF(\"1/1/2020\", \"1/1/2021\", \"Y\")");
    model._set("A2", "=DATEDIF(\"1/1/2020\", \"6/15/2021\", \"M\")");
    model._set("A3", "=DATEDIF(\"1/1/2020\", \"1/2/2020\", \"D\")");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"17");
    assert_eq!(model._get_text("A3"), *"1");
}
