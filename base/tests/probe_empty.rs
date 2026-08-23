use ironcalc_base::Model;
#[test]
fn empty_passthrough_semantics() {
    let mut x = Model::new_empty("t", "en", "UTC", "en").unwrap();
    x.set_user_input(0, 2, 1, "=A1".to_string()).unwrap(); // A2 = A1 (blank)
    x.set_user_input(0, 3, 1, "=ISBLANK(A2)".to_string())
        .unwrap();
    x.set_user_input(0, 4, 1, "=COUNT(A2)".to_string()).unwrap();
    x.set_user_input(0, 5, 1, "=COUNTBLANK(A2)".to_string())
        .unwrap();
    x.set_user_input(0, 6, 1, "=A2+1".to_string()).unwrap();
    x.set_user_input(0, 7, 1, "=A2&\"x\"".to_string()).unwrap();
    x.evaluate();
    for (r, what) in [
        (2, "=A1 display"),
        (3, "ISBLANK(A2)"),
        (4, "COUNT(A2)"),
        (5, "COUNTBLANK(A2)"),
        (6, "A2+1"),
        (7, "A2&\"x\""),
    ] {
        println!(
            "{what:<16} formatted={:?} value={:?}",
            x.get_formatted_cell_value(0, r, 1).unwrap(),
            x.get_cell_value_by_index(0, r, 1).unwrap()
        );
    }
}
