//! Seeds 20/23 regression: a formula written while a name is undefined parses
//! as a scalar `NamedVariable`; once the name is defined as a *range*, the
//! reparse must wrap it in the implicit-intersection operator, or a 1×N array
//! reaches a scalar-context cell and trips the debug guard.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ironcalc_base::{Model, RecalcMode};

#[test]
fn name_defined_as_range_after_write_stays_scalar() {
    let run = |mode: RecalcMode| -> Result<String, String> {
        let model = Model::new_empty("m", "en", "UTC", "en")?;
        let mut model = model.with_recalc_mode(mode);
        model.set_user_input(0, 6, 4, "=NRANGE".to_string())?;
        model
            .new_defined_name("NRANGE", None, "Sheet1!$A$1:$A$8")
            .map_err(|e| format!("new_defined_name: {e}"))?;
        model.evaluate();
        model.get_formatted_cell_value(0, 6, 4)
    };
    let full = run(RecalcMode::Full).unwrap();
    let incremental = run(RecalcMode::Incremental).unwrap();
    println!("F6 full={full} incr={incremental}");
    assert_eq!(full, incremental);
    // Implicit intersection against A1:A8 from row 6 picks A6 (empty -> 0).
    assert_eq!(full, "0");
}

#[test]
fn renamed_name_to_range_stays_scalar() {
    // seed 23 shape: the formula references the name under its old identity.
    let model = Model::new_empty("m", "en", "UTC", "en").unwrap();
    let mut model = model.with_recalc_mode(RecalcMode::Full);
    model
        .set_user_input(0, 12, 6, "=NDATA".to_string())
        .unwrap();
    model
        .new_defined_name("NDATA", None, "Sheet1!$A$2")
        .unwrap();
    model
        .update_defined_name("NDATA", None, "NRANGE", None, "Sheet1!$E$1:$E$5")
        .unwrap();
    model.evaluate();
    let v = model.get_formatted_cell_value(0, 12, 6).unwrap();
    println!("F12={v}");
    // II of E1:E5 from row 12 has no intersection: #VALUE! per the classic
    // rules. Either way both modes must agree and no panic may fire.
    assert_eq!(v, "#VALUE!");
}
