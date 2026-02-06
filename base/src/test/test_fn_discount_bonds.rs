#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn fn_pricedisc() {
    let mut model = new_empty_model();
    model._set("A2", "=DATE(2022,1,25)");
    model._set("A3", "=DATE(2022,11,15)");
    model._set("A4", "3.75%");
    model._set("A5", "100");

    model._set("B1", "=PRICEDISC(A2,A3,A4,A5)");
    model._set("C1", "=PRICEDISC(A2,A3)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), "96.979166667");
    assert_eq!(model._get_text("C1"), *"#ERROR!");
}

#[test]
fn fn_yielddisc() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2022,1,25)");
    model._set("A2", "=DATE(2022,11,15)");
    model._set("A3", "97");
    model._set("A4", "100");

    model._set("B1", "=YIELDDISC(A1,A2,A3,A4)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), "0.038393175");
}

#[test]
fn fn_disc() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2022,1,25)");
    model._set("A2", "=DATE(2022,11,15)");
    model._set("A3", "97");
    model._set("A4", "100");

    model._set("B1", "=DISC(A1,A2,A3,A4)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), "0.037241379");
}

#[test]
fn fn_received() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2020,1,1)");
    model._set("A2", "=DATE(2023,6,30)");
    model._set("A3", "20000");
    model._set("A4", "5%");
    model._set("A5", "3");

    model._set("B1", "=RECEIVED(A1,A2,A3,A4,A5)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), "24236.387782205");
}

#[test]
fn fn_intrate() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2020,1,1)");
    model._set("A2", "=DATE(2023,6,30)");
    model._set("A3", "10000");
    model._set("A4", "12000");
    model._set("A5", "3");

    model._set("B1", "=INTRATE(A1,A2,A3,A4,A5)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), "0.057210031");
}

#[test]
fn fn_discount_bond_argument_errors() {
    let mut model = new_empty_model();

    model._set("A1", "=PRICEDISC()");
    model._set("A2", "=PRICEDISC(1,2,3)");
    model._set("A3", "=PRICEDISC(1,2,3,4,5,6)");

    model._set("B1", "=YIELDDISC()");
    model._set("B2", "=YIELDDISC(1,2,3)");
    model._set("B3", "=YIELDDISC(1,2,3,4,5,6)");

    model._set("C1", "=DISC()");
    model._set("C2", "=DISC(1,2,3)");
    model._set("C3", "=DISC(1,2,3,4,5,6)");

    model._set("D1", "=RECEIVED()");
    model._set("D2", "=RECEIVED(1,2,3)");
    model._set("D3", "=RECEIVED(1,2,3,4,5,6)");

    model._set("E1", "=INTRATE()");
    model._set("E2", "=INTRATE(1,2,3)");
    model._set("E3", "=INTRATE(1,2,3,4,5,6)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("A3"), *"#ERROR!");
    assert_eq!(model._get_text("B1"), *"#ERROR!");
    assert_eq!(model._get_text("B2"), *"#ERROR!");
    assert_eq!(model._get_text("B3"), *"#ERROR!");
    assert_eq!(model._get_text("C1"), *"#ERROR!");
    assert_eq!(model._get_text("C2"), *"#ERROR!");
    assert_eq!(model._get_text("C3"), *"#ERROR!");
    assert_eq!(model._get_text("D1"), *"#ERROR!");
    assert_eq!(model._get_text("D2"), *"#ERROR!");
    assert_eq!(model._get_text("D3"), *"#ERROR!");
    assert_eq!(model._get_text("E1"), *"#ERROR!");
    assert_eq!(model._get_text("E2"), *"#ERROR!");
    assert_eq!(model._get_text("E3"), *"#ERROR!");
}

#[test]
fn fn_discount_bond_parameter_validation() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2022,1,1)");
    model._set("A2", "=DATE(2022,12,31)");

    model._set("B1", "=PRICEDISC(A1,A2,0.05,0)");
    model._set("B2", "=PRICEDISC(A1,A2,0,100)");
    model._set("B3", "=PRICEDISC(A1,A2,-0.05,100)");

    model._set("C1", "=YIELDDISC(A1,A2,0,100)");
    model._set("C2", "=YIELDDISC(A1,A2,95,0)");
    model._set("C3", "=YIELDDISC(A1,A2,-95,100)");

    model._set("D1", "=DISC(A1,A2,0,100)");
    model._set("D2", "=DISC(A1,A2,95,0)");
    model._set("D3", "=DISC(A1,A2,-95,100)");

    model._set("E1", "=RECEIVED(A1,A2,0,0.05)");
    model._set("E2", "=RECEIVED(A1,A2,1000,0)");
    model._set("E3", "=RECEIVED(A1,A2,-1000,0.05)");

    model._set("F1", "=INTRATE(A1,A2,0,1050)");
    model._set("F2", "=INTRATE(A1,A2,1000,0)");
    model._set("F3", "=INTRATE(A1,A2,-1000,1050)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"#NUM!");
    assert_eq!(model._get_text("B2"), *"#NUM!");
    assert_eq!(model._get_text("C1"), *"#NUM!");
    assert_eq!(model._get_text("C2"), *"#NUM!");
}

#[test]
fn fn_discount_bond_basis_validation() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2022,1,1)");
    model._set("A2", "=DATE(2022,12,31)");

    model._set("B1", "=PRICEDISC(A1,A2,0.05,100,0)");
    model._set("B2", "=PRICEDISC(A1,A2,0.05,100,1)");
    model._set("B3", "=PRICEDISC(A1,A2,0.05,100,2)");
    model._set("B4", "=PRICEDISC(A1,A2,0.05,100,3)");
    model._set("B5", "=PRICEDISC(A1,A2,0.05,100,4)");

    model._set("C1", "=PRICEDISC(A1,A2,0.05,100,-1)");
    model._set("C2", "=PRICEDISC(A1,A2,0.05,100,5)");
    model._set("C3", "=YIELDDISC(A1,A2,95,100,10)");
    model._set("C4", "=DISC(A1,A2,95,100,-5)");
    model._set("C5", "=RECEIVED(A1,A2,1000,0.05,99)");
    model._set("C6", "=INTRATE(A1,A2,1000,1050,-2)");

    model.evaluate();

    assert_ne!(model._get_text("B1"), *"#ERROR!");
    assert_ne!(model._get_text("B2"), *"#ERROR!");
    assert_ne!(model._get_text("B3"), *"#ERROR!");
    assert_ne!(model._get_text("B4"), *"#ERROR!");
    assert_ne!(model._get_text("B5"), *"#ERROR!");

    assert_eq!(model._get_text("C1"), *"#NUM!");
    assert_eq!(model._get_text("C2"), *"#NUM!");
    assert_eq!(model._get_text("C3"), *"#NUM!");
    assert_eq!(model._get_text("C4"), *"#NUM!");
    assert_eq!(model._get_text("C5"), *"#NUM!");
    assert_eq!(model._get_text("C6"), *"#NUM!");
}
