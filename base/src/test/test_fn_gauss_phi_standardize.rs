#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_gauss() {
    let mut model = new_empty_model();
    model._set("A1", "=GAUSS(2)");
    model._set("A2", "=GAUSS(0)");
    model.evaluate();
    let a1: f64 = model._get_text("A1").parse().unwrap();
    let a2: f64 = model._get_text("A2").parse().unwrap();
    assert!((a1 - 0.477249868).abs() < 1e-9);
    assert!(a2.abs() < 1e-12);
}

#[test]
fn test_phi() {
    let mut model = new_empty_model();
    model._set("A1", "=PHI(0)");
    model._set("A2", "=PHI(1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0.39894228");
    assert_eq!(model._get_text("A2"), *"0.241970725");
}

#[test]
fn test_standardize() {
    let mut model = new_empty_model();
    model._set("A1", "=STANDARDIZE(75,70,5)");
    model._set("A2", "=STANDARDIZE(65,70,5)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"-1");
}
