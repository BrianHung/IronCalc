#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn fn_pricemat() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2019,2,15)");
    model._set("A2", "=DATE(2025,4,13)");
    model._set("A3", "=DATE(2018,11,11)");
    model._set("A4", "5.75%");
    model._set("A5", "6.5%");

    model._set("B1", "=PRICEMAT(A1,A2,A3,A4,A5)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), "96.271187821");
}

#[test]
fn fn_yieldmat() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2019,2,15)");
    model._set("A2", "=DATE(2025,4,13)");
    model._set("A3", "=DATE(2018,11,11)");
    model._set("A4", "5.75%");
    model._set("A5", "96.27");

    model._set("B1", "=YIELDMAT(A1,A2,A3,A4,A5)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), "0.065002762");
}

#[test]
fn fn_pricemat_yieldmat_argument_errors() {
    let mut model = new_empty_model();

    model._set("A1", "=PRICEMAT()");
    model._set("A2", "=PRICEMAT(1,2,3,4)");
    model._set("A3", "=PRICEMAT(1,2,3,4,5,6,7)");

    model._set("B1", "=YIELDMAT()");
    model._set("B2", "=YIELDMAT(1,2,3,4)");
    model._set("B3", "=YIELDMAT(1,2,3,4,5,6,7)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("A3"), *"#ERROR!");
    assert_eq!(model._get_text("B1"), *"#ERROR!");
    assert_eq!(model._get_text("B2"), *"#ERROR!");
    assert_eq!(model._get_text("B3"), *"#ERROR!");
}

#[test]
fn fn_pricemat_yieldmat_date_ordering() {
    let mut model = new_empty_model();

    model._set("A1", "=DATE(2022,1,1)"); // settlement
    model._set("A2", "=DATE(2021,12,31)"); // maturity (before settlement)
    model._set("A3", "=DATE(2020,1,1)"); // issue
    model._set("A4", "=DATE(2023,1,1)"); // later issue date

    model._set("B1", "=PRICEMAT(A1,A2,A3,0.06,0.05)");
    model._set("B2", "=YIELDMAT(A1,A2,A3,0.06,99)");
    model._set("C1", "=PRICEMAT(A1,A2,A4,0.06,0.05)");
    model._set("C2", "=YIELDMAT(A1,A2,A4,0.06,99)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"#NUM!");
    assert_eq!(model._get_text("B2"), *"#NUM!");
    assert_eq!(model._get_text("C1"), *"#NUM!");
    assert_eq!(model._get_text("C2"), *"#NUM!");
}

#[test]
fn fn_pricemat_yieldmat_parameter_validation() {
    let mut model = new_empty_model();

    model._set("A1", "=DATE(2022,1,1)");
    model._set("A2", "=DATE(2022,12,31)");
    model._set("A3", "=DATE(2021,1,1)");

    model._set("B1", "=PRICEMAT(A1,A2,A3,-0.06,0.05)");
    model._set("B2", "=PRICEMAT(A1,A2,A3,0.06,-0.05)");

    model._set("C1", "=YIELDMAT(A1,A2,A3,0.06,0)");
    model._set("C2", "=YIELDMAT(A1,A2,A3,-0.06,99)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"#NUM!");
    assert_eq!(model._get_text("B2"), *"#NUM!");
    assert_eq!(model._get_text("C1"), *"#NUM!");
    assert_eq!(model._get_text("C2"), *"#NUM!");
}
