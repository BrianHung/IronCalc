#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn fn_price_yield() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2023,1,1)");
    model._set("A2", "=DATE(2024,1,1)");
    model._set("A3", "5%");

    model._set("B1", "=PRICE(A1,A2,A3,6%,100,1)");
    model._set("B2", "=YIELD(A1,A2,A3,B1,100,1)");

    model.evaluate();
    assert_eq!(model._get_text("B1"), "99.056603774");
    assert_eq!(model._get_text("B2"), "0.06");
}

#[test]
fn fn_price_frequencies() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2023,1,1)");
    model._set("A2", "=DATE(2024,1,1)");

    model._set("B1", "=PRICE(A1,A2,5%,6%,100,1)");
    model._set("B2", "=PRICE(A1,A2,5%,6%,100,2)");
    model._set("B3", "=PRICE(A1,A2,5%,6%,100,4)");

    model.evaluate();

    let annual: f64 = model._get_text("B1").parse().unwrap();
    let semi: f64 = model._get_text("B2").parse().unwrap();
    let quarterly: f64 = model._get_text("B3").parse().unwrap();

    assert_ne!(annual, semi);
    assert_ne!(semi, quarterly);
}

#[test]
fn fn_yield_frequencies() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2023,1,1)");
    model._set("A2", "=DATE(2024,1,1)");

    model._set("B1", "=YIELD(A1,A2,5%,99,100,1)");
    model._set("B2", "=YIELD(A1,A2,5%,99,100,2)");
    model._set("B3", "=YIELD(A1,A2,5%,99,100,4)");

    model.evaluate();

    let annual: f64 = model._get_text("B1").parse().unwrap();
    let semi: f64 = model._get_text("B2").parse().unwrap();
    let quarterly: f64 = model._get_text("B3").parse().unwrap();

    assert_ne!(annual, semi);
    assert_ne!(semi, quarterly);
}

#[test]
fn fn_price_argument_errors() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2023,1,1)");
    model._set("A2", "=DATE(2024,1,1)");

    model._set("B1", "=PRICE()");
    model._set("B2", "=PRICE(A1,A2,5%,6%,100)");
    model._set("B3", "=PRICE(A1,A2,5%,6%,100,2,0,99)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"#ERROR!");
    assert_eq!(model._get_text("B2"), *"#ERROR!");
    assert_eq!(model._get_text("B3"), *"#ERROR!");
}

#[test]
fn fn_yield_argument_errors() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2023,1,1)");
    model._set("A2", "=DATE(2024,1,1)");

    model._set("B1", "=YIELD()");
    model._set("B2", "=YIELD(A1,A2,5%,99,100)");
    model._set("B3", "=YIELD(A1,A2,5%,99,100,2,0,99)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"#ERROR!");
    assert_eq!(model._get_text("B2"), *"#ERROR!");
    assert_eq!(model._get_text("B3"), *"#ERROR!");
}

#[test]
fn fn_price_invalid_frequency() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2023,1,1)");
    model._set("A2", "=DATE(2024,1,1)");

    model._set("B1", "=PRICE(A1,A2,5%,6%,100,0)");
    model._set("B2", "=PRICE(A1,A2,5%,6%,100,3)");
    model._set("B3", "=PRICE(A1,A2,5%,6%,100,5)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"#ERROR!");
    assert_eq!(model._get_text("B2"), *"#ERROR!");
    assert_eq!(model._get_text("B3"), *"#ERROR!");
}

#[test]
fn fn_yield_invalid_frequency() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2023,1,1)");
    model._set("A2", "=DATE(2024,1,1)");

    model._set("B1", "=YIELD(A1,A2,5%,99,100,0)");
    model._set("B2", "=YIELD(A1,A2,5%,99,100,3)");
    model._set("B3", "=YIELD(A1,A2,5%,99,100,5)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"#ERROR!");
    assert_eq!(model._get_text("B2"), *"#ERROR!");
    assert_eq!(model._get_text("B3"), *"#ERROR!");
}

#[test]
fn fn_price_invalid_dates() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2023,1,1)");
    model._set("A2", "=DATE(2024,1,1)");

    model._set("B1", "=PRICE(A2,A1,5%,6%,100,2)");
    model._set("B2", "=PRICE(A1,A1,5%,6%,100,2)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"#ERROR!");
    assert_eq!(model._get_text("B2"), *"#ERROR!");
}

#[test]
fn fn_yield_invalid_dates() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2023,1,1)");
    model._set("A2", "=DATE(2024,1,1)");

    model._set("B1", "=YIELD(A2,A1,5%,99,100,2)");
    model._set("B2", "=YIELD(A1,A1,5%,99,100,2)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"#ERROR!");
    assert_eq!(model._get_text("B2"), *"#ERROR!");
}

#[test]
fn fn_price_with_basis() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2023,1,1)");
    model._set("A2", "=DATE(2024,1,1)");

    model._set("B1", "=PRICE(A1,A2,5%,6%,100,2,0)");
    model._set("B2", "=PRICE(A1,A2,5%,6%,100,2,1)");

    model.evaluate();

    assert!(model._get_text("B1").parse::<f64>().is_ok());
    assert!(model._get_text("B2").parse::<f64>().is_ok());
}

#[test]
fn fn_yield_with_basis() {
    let mut model = new_empty_model();
    model._set("A1", "=DATE(2023,1,1)");
    model._set("A2", "=DATE(2024,1,1)");

    model._set("B1", "=YIELD(A1,A2,5%,99,100,2,0)");
    model._set("B2", "=YIELD(A1,A2,5%,99,100,2,1)");

    model.evaluate();

    assert!(model._get_text("B1").parse::<f64>().is_ok());
    assert!(model._get_text("B2").parse::<f64>().is_ok());
}
