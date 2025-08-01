#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_covariance_functions() {
    let mut model = new_empty_model();
    let vals1 = [3,2,4,5,6];
    let vals2 = [9,7,12,15,17];
    for (i,v) in vals1.iter().enumerate() { model._set(&format!("B{}", i+1), &v.to_string()); }
    for (i,v) in vals2.iter().enumerate() { model._set(&format!("C{}", i+1), &v.to_string()); }
    model._set("A1", "=COVARIANCE.P(B1:B5,C1:C5)");
    model._set("A2", "=COVARIANCE.S(B1:B5,C1:C5)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"5.2");
    assert_eq!(model._get_text("A2"), *"6.5");
}
