use crate::test::util::new_empty_model;

#[test]
fn test_networkdays_basic() {
    let mut model = new_empty_model();
    model._set("A1", "=NETWORKDAYS(44927,44936)");
    model._set("A2", "=NETWORKDAYS(44927,44936,44932)");
    model._set("A3", "=NETWORKDAYS(44936,44927)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"7");
    assert_eq!(model._get_text("A2"), *"6");
    assert_eq!(model._get_text("A3"), *"-7");
}

#[test]
fn test_networkdays_intl_basic() {
    let mut model = new_empty_model();
    model._set("A1", "=NETWORKDAYS.INTL(44927,44936,11)");
    model._set("A2", "=NETWORKDAYS.INTL(44927,44928,\"1111100\")");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"8");
    assert_eq!(model._get_text("A2"), *"1");
}
