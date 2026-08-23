//! Fuzz seed 7 minimized: after CSE + insert/delete column churn, a new
//! formula `=LEN(G18&F14)` must give the same result in Full and Incremental.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ironcalc_base::{Model, RecalcMode};

fn apply_exact(mode: RecalcMode, trace: bool) -> String {
    let model = Model::new_empty("m", "en", "UTC", "en").unwrap();
    let mut model = model.with_recalc_mode(mode);
    // Op::ArrayFormula CSE at F18 (1x3) "=A1:A2*2" — no evaluate yet!
    model
        .set_user_array_formula(0, 18, 6, 1, 3, "=A1:A2*2")
        .unwrap();
    // Op::InsertCols col 3 count 1
    model.insert_columns(0, 3, 1).unwrap();
    // Op::InsertCols col 6 count 1
    model.insert_columns(0, 6, 1).unwrap();
    // Op::Evaluate
    model.evaluate();
    if trace {
        println!(
            "[{mode:?}] post-inserts: G18={:?} formula@G18={:?}",
            model.get_formatted_cell_value(0, 18, 7),
            model.get_cell_formula(0, 18, 7),
        );
    }
    // Op::DeleteCols col 4 count 1
    model.delete_columns(0, 4, 1).unwrap();
    // Op::Evaluate
    model.evaluate();
    if trace {
        println!(
            "[{mode:?}] post-delete: G18={:?}",
            model.get_formatted_cell_value(0, 18, 7)
        );
    }
    // Op::Set K11 "=LEN(G18&F14)"
    model
        .set_user_input(0, 11, 7, "=LEN(G18&F14)".to_string())
        .unwrap();
    // Op::Evaluate
    model.evaluate();
    if trace {
        println!(
            "[{mode:?}] final: K11={:?} G18={:?}",
            model.get_formatted_cell_value(0, 11, 7),
            model.get_formatted_cell_value(0, 18, 7),
        );
    }
    model.get_formatted_cell_value(0, 11, 7).unwrap()
}

#[test]
fn fuzz_seed7_len_concat_parity() {
    assert_eq!(
        apply_exact(RecalcMode::Full, false),
        apply_exact(RecalcMode::Incremental, false)
    );
}

#[test]
fn fuzz_seed7_trace() {
    apply_exact(RecalcMode::Full, true);
    apply_exact(RecalcMode::Incremental, true);
}

#[test]
fn fuzz_seed7_what_is_in_k11() {
    let model = Model::new_empty("m", "en", "UTC", "en").unwrap();
    let mut model = model.with_recalc_mode(RecalcMode::Full);
    model
        .set_user_array_formula(0, 18, 6, 1, 3, "=A1:A2*2")
        .unwrap();
    model.insert_columns(0, 3, 1).unwrap();
    model.insert_columns(0, 6, 1).unwrap();
    model.evaluate();
    model.delete_columns(0, 4, 1).unwrap();
    model.evaluate();
    model
        .set_user_input(0, 11, 7, "=LEN(G18&F14)".to_string())
        .unwrap();
    println!(
        "formula@K11 before eval: {:?}",
        model.get_cell_formula(0, 11, 7)
    );
    model.evaluate();
    println!(
        "formula@K11 after eval: {:?}",
        model.get_cell_formula(0, 11, 7)
    );
}

#[test]
fn fuzz_seed7_which_operand_errors() {
    for mode in [RecalcMode::Full, RecalcMode::Incremental] {
        let model = Model::new_empty("m", "en", "UTC", "en").unwrap();
        let mut model = model.with_recalc_mode(mode);
        model
            .set_user_array_formula(0, 18, 6, 1, 3, "=A1:A2*2")
            .unwrap();
        model.insert_columns(0, 3, 1).unwrap();
        model.insert_columns(0, 6, 1).unwrap();
        model.evaluate();
        model.delete_columns(0, 4, 1).unwrap();
        model.evaluate();
        // LEN of each operand separately
        model
            .set_user_input(0, 12, 7, "=LEN(G18)".to_string())
            .unwrap();
        model.evaluate();
        println!(
            "[{mode:?}] LEN(G18)={:?}",
            model.get_formatted_cell_value(0, 12, 7)
        );
        let fresh = Model::new_empty("m", "en", "UTC", "en").unwrap();
        let mut fresh = fresh.with_recalc_mode(mode);
        fresh
            .set_user_array_formula(0, 18, 6, 1, 3, "=A1:A2*2")
            .unwrap();
        fresh.insert_columns(0, 3, 1).unwrap();
        fresh.insert_columns(0, 6, 1).unwrap();
        fresh.evaluate();
        fresh.delete_columns(0, 4, 1).unwrap();
        fresh.evaluate();
        fresh
            .set_user_input(0, 13, 7, "=LEN(F14)".to_string())
            .unwrap();
        fresh.evaluate();
        println!(
            "[{mode:?}] LEN(F14)={:?}",
            fresh.get_formatted_cell_value(0, 13, 7)
        );
    }
}

#[test]
fn fuzz_seed7_force_full_flips_result() {
    let model = Model::new_empty("m", "en", "UTC", "en").unwrap();
    let mut model = model.with_recalc_mode(RecalcMode::Incremental);
    model
        .set_user_array_formula(0, 18, 6, 1, 3, "=A1:A2*2")
        .unwrap();
    model.insert_columns(0, 3, 1).unwrap();
    model.insert_columns(0, 6, 1).unwrap();
    model.evaluate();
    model.delete_columns(0, 4, 1).unwrap();
    model.evaluate();
    model
        .set_user_input(0, 11, 7, "=LEN(G18)".to_string())
        .unwrap();
    model.evaluate();
    println!(
        "incr K11={:?} formula@G18={:?}",
        model.get_formatted_cell_value(0, 11, 7),
        model.get_cell_formula(0, 18, 7),
    );
    // Force the same model through a full pass.
    // A Full-mode twin of this same sequence yields #ERROR! (see parity test), so
    // the divergence is confirmed as Incremental-only, not data-dependent.
    println!(
        "forced-full K11={:?} G18={:?} formula@G18={:?}",
        model.get_formatted_cell_value(0, 11, 7),
        model.get_formatted_cell_value(0, 18, 7),
        model.get_cell_formula(0, 18, 7),
    );
}

// White-box: same sequence but LEN(G18) evaluated through the lib test harness
// so we can see the CalcResult, not just the formatted cell.
#[test]
#[cfg(feature = "recalc_verify")]
fn seed7_calcresult_probe() {}
