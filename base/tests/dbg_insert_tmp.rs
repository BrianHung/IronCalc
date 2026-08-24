#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use ironcalc_base::Model;
#[test]
fn dbg() {
    let mut m = Model::new_empty("t", "en", "UTC", "en").unwrap();
    m.set_user_array_formula(0, 3, 2, 2, 3, "=1+1").unwrap();
    m.evaluate();
    let r = m.insert_columns(0, 1, 1);
    println!("insert_columns -> {r:?}");
    if r.is_err() {
        panic!("boom");
    }
}
