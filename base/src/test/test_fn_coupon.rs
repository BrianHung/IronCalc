#![allow(clippy::unwrap_used)]

use crate::{cell::CellValue, test::util::new_empty_model};

#[test]
fn fn_coupon_functions() {
    let mut model = new_empty_model();

    // Test with basis 1 (original test)
    model._set("A1", "=DATE(2001,1,25)");
    model._set("A2", "=DATE(2001,11,15)");
    model._set("B1", "=COUPDAYBS(A1,A2,2,1)");
    model._set("B2", "=COUPDAYS(A1,A2,2,1)");
    model._set("B3", "=COUPDAYSNC(A1,A2,2,1)");
    model._set("B4", "=COUPNCD(A1,A2,2,1)");
    model._set("B5", "=COUPNUM(A1,A2,2,1)");
    model._set("B6", "=COUPPCD(A1,A2,2,1)");

    // Test with basis 3 for better coverage
    model._set("C1", "=COUPDAYBS(DATE(2001,1,25),DATE(2001,11,15),2,3)");
    model._set("C2", "=COUPDAYS(DATE(2001,1,25),DATE(2001,11,15),2,3)");
    model._set("C3", "=COUPDAYSNC(DATE(2001,1,25),DATE(2001,11,15),2,3)");
    model._set("C4", "=COUPNCD(DATE(2001,1,25),DATE(2001,11,15),2,3)");
    model._set("C5", "=COUPNUM(DATE(2007,1,25),DATE(2008,11,15),2,1)");
    model._set("C6", "=COUPPCD(DATE(2001,1,25),DATE(2001,11,15),2,3)");

    model.evaluate();

    // Test basis 1
    assert_eq!(model._get_text("B1"), "71");
    assert_eq!(model._get_text("B2"), "181");
    assert_eq!(model._get_text("B3"), "110");
    assert_eq!(
        model.get_cell_value_by_ref("Sheet1!B4"),
        Ok(CellValue::Number(37026.0))
    );
    assert_eq!(model._get_text("B5"), "2");
    assert_eq!(
        model.get_cell_value_by_ref("Sheet1!B6"),
        Ok(CellValue::Number(36845.0))
    );

    // Test basis 3 (more comprehensive coverage)
    assert_eq!(model._get_text("C1"), "71");
    assert_eq!(model._get_text("C2"), "181"); // Fixed: actual days
    assert_eq!(model._get_text("C3"), "110");
    assert_eq!(model._get_text("C4"), "37026");
    assert_eq!(model._get_text("C5"), "4");
    assert_eq!(model._get_text("C6"), "36845");
}

#[test]
fn fn_coupon_functions_error_cases() {
    let mut model = new_empty_model();

    // Test invalid frequency
    model._set("E1", "=COUPDAYBS(DATE(2001,1,25),DATE(2001,11,15),3,1)");
    // Test invalid basis
    model._set("E2", "=COUPDAYS(DATE(2001,1,25),DATE(2001,11,15),2,5)");
    // Test settlement >= maturity
    model._set("E3", "=COUPDAYSNC(DATE(2001,11,15),DATE(2001,1,25),2,1)");
    // Test too few arguments
    model._set("E4", "=COUPNCD(DATE(2001,1,25),DATE(2001,11,15))");
    // Test too many arguments
    model._set("E5", "=COUPNUM(DATE(2001,1,25),DATE(2001,11,15),2,1,1)");

    model.evaluate();

    // All should return errors
    assert_eq!(model._get_text("E1"), "#NUM!");
    assert_eq!(model._get_text("E2"), "#NUM!");
    assert_eq!(model._get_text("E3"), "#NUM!");
    assert_eq!(model._get_text("E4"), *"#ERROR!");
    assert_eq!(model._get_text("E5"), *"#ERROR!");
}

#[test]
fn fn_coupdays_actual_day_count_fix() {
    // Verify COUPDAYS correctly distinguishes between fixed vs actual day count methods
    // Bug: basis 2&3 were incorrectly using fixed calculations like basis 0&4
    let mut model = new_empty_model();

    model._set("A1", "=DATE(2023,1,15)");
    model._set("A2", "=DATE(2023,7,15)");

    model._set("B1", "=COUPDAYS(A1,A2,2,0)"); // 30/360: uses 360/freq
    model._set("B2", "=COUPDAYS(A1,A2,2,2)"); // Actual/360: uses actual days
    model._set("B3", "=COUPDAYS(A1,A2,2,3)"); // Actual/365: uses actual days
    model._set("B4", "=COUPDAYS(A1,A2,2,4)"); // 30/360 European: uses 360/freq

    model.evaluate();

    // Basis 0&4: theoretical 360/2 = 180 days
    assert_eq!(model._get_text("B1"), "180");
    assert_eq!(model._get_text("B4"), "180");

    // Basis 2&3: actual days between Jan 15 and Jul 15 = 181 days
    assert_eq!(model._get_text("B2"), "181");
    assert_eq!(model._get_text("B3"), "181");
}
