#![allow(clippy::unwrap_used)]

use crate::test::util::{incremental_mode, new_empty_model};
use crate::UserModel;

#[test]
fn incremental_matches_full_on_a_chain() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=A1*2");
    model._set("C1", "=B1+1");
    model.evaluate();

    model._set("A1", "10");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "20");
    assert_eq!(model._get_text("C1"), "21");
}

#[test]
fn volatile_hidden_in_defined_name_lambda_is_detected() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    // A lambda stored as a defined name, hiding a volatile function. Passed bare
    // to MAP it appears as a defined-name node, not an inline lambda.
    model
        .new_defined_name("V", None, "=LAMBDA(x, x + RAND())")
        .unwrap();
    model._set("A1", "1");
    model._set("B1", "=MAP(A1, V)");
    model.evaluate();

    // B1 (sheet 0, row 1, column 2) must be volatile so the incremental path
    // re-rolls it every pass, matching a full recompute.
    assert!(model.graph.volatile.contains(&(0, 1, 2)));
}

#[test]
fn incremental_coexists_with_arrays() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("A1", "=B1:B2"); // spills A1:A2
    model._set("D1", "10"); // independent
    model._set("D2", "=D1+1");
    model.evaluate();

    model._set("D1", "20"); // unrelated to the spill
    model.evaluate();
    assert_eq!(model._get_text("D2"), "21");
    assert_eq!(model._get_text("A2"), "2"); // spill untouched

    model._set("B2", "9"); // feeds the spill: falls back to full
    model.evaluate();
    assert_eq!(model._get_text("A2"), "9"); // re-spilled
}

#[test]
fn incremental_handles_row_insert() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    for row in 1..=5 {
        model._set(&format!("A{row}"), &row.to_string());
    }
    model._set("C1", "=SUM(A1:A5)");
    model._set("E1", "=ROWS(A1:A5)");
    model.evaluate();

    // An insert only shifts references, so the graph shifts its edges rather
    // than rebuilding: the next pass stays incremental.
    model.insert_rows(0, 3, 2).unwrap(); // A1:A5 -> A1:A7
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("C1"), "15"); // SUM unchanged
    assert_eq!(model._get_text("E1"), "7"); // ROWS grew
}

#[test]
fn incremental_handles_row_delete() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "10");
    model._set("A5", "=A1+1");
    model.evaluate();

    // No tracked range straddles the deleted row, so the shift models it and the
    // pass stays incremental.
    model.delete_rows(0, 2, 1).unwrap(); // A5 -> A4, ref A1 intact
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("A4"), "11");
}

#[test]
fn incremental_column_insert_stays_incremental() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "2");
    model._set("D1", "=A1+B1");
    model.evaluate();

    model.insert_columns(0, 2, 1).unwrap(); // B shifts to C, D to E
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("E1"), "3");
}

#[test]
fn incremental_row_delete_shrinking_range_forces_full() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    for row in 1..=5 {
        model._set(&format!("A{row}"), &row.to_string());
    }
    model._set("C1", "=SUM(A1:A5)");
    model.evaluate();

    // Deleting a row inside the summed range shrinks it; the shift cannot model a
    // partial-range removal, so it falls back to a full recompute rather than
    // leave a stale sum.
    model.delete_rows(0, 3, 1).unwrap(); // drops A3, A1:A5 -> A1:A4
    assert!(model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("C1"), "12"); // 1 + 2 + 4 + 5
}

#[test]
fn incremental_structural_edit_moves_volatile_with_the_graph() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A10", "=RAND()");
    model.evaluate();
    assert!(model.graph.volatile.contains(&(0, 10, 1)));

    // The graph shifts every position-keyed set, so the volatile travels to A11
    // and the next pass can stay incremental.
    model.insert_rows(0, 1, 1).unwrap();
    assert!(!model.graph.should_recompute_full());
    assert!(model.graph.volatile.contains(&(0, 11, 1)));
    assert!(!model.graph.volatile.contains(&(0, 10, 1)));

    model.evaluate();
    assert!(model.graph.volatile.contains(&(0, 11, 1)));
}

#[test]
fn incremental_insert_moves_hyperlink_with_the_cell() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A10", "=HYPERLINK(\"http://a.com\",\"x\")");
    model.evaluate();
    let dynamic_rows = |model: &crate::Model| {
        model
            .get_links_list(0)
            .unwrap()
            .into_iter()
            .filter(|l| l.dynamic)
            .map(|l| l.row)
            .collect::<Vec<_>>()
    };
    assert_eq!(dynamic_rows(&model), vec![10]);

    model.insert_rows(0, 1, 1).unwrap();
    // The formula map must move with the cell; a leftover at A10 is a ghost.
    assert_eq!(dynamic_rows(&model), vec![11]);
    model.evaluate();
    assert_eq!(dynamic_rows(&model), vec![11]);
}

#[test]
fn incremental_structural_edit_below_volatile_stays_incremental() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=RAND()");
    model._set("C10", "=1+1");
    model.evaluate();

    // Inserting below the volatile leaves it in place, so the edit can stay
    // incremental.
    model.insert_rows(0, 5, 1).unwrap();
    assert!(!model.graph.should_recompute_full());
}

#[test]
fn incremental_shares_one_range_vertex() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    for row in 1..=10 {
        model._set(&format!("A{row}"), &row.to_string());
    }
    for row in 1..=50 {
        model._set(&format!("C{row}"), "=SUM(A1:A10)");
    }
    model.evaluate();

    model._set("A5", "100"); // sum 55 -> 150
    model.evaluate();
    for row in 1..=50 {
        assert_eq!(model._get_text(&format!("C{row}")), "150");
    }
}

#[test]
fn incremental_recomputes_volatiles() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "5");
    model._set("A1", "=INDIRECT(\"C1\")"); // reads C1 dynamically
    model.evaluate();
    assert_eq!(model._get_text("A1"), "5");

    model._set("C1", "99"); // A1's hidden target; without volatiles A1 stays 5
    model.evaluate();
    assert_eq!(model._get_text("A1"), "99");
}

#[test]
fn incremental_wide_fanout_stays_correct() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    // Exceed INCREMENTAL_FANOUT_FLOOR (1024) so editing A1 trips the fanout
    // guard and falls back to full; the result must still be correct.
    for row in 1..=1100 {
        model._set(&format!("C{row}"), "=$A$1*2");
    }
    model.evaluate();

    model._set("A1", "5");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "10");
    assert_eq!(model._get_text("C1100"), "10");
}

#[test]
fn incremental_tracks_defined_name_references() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    let sheet = model.workbook.worksheets[0].get_name();
    model
        .new_defined_name("MyRef", None, &format!("{sheet}!$B$1"))
        .unwrap();
    model._set("B1", "5");
    model._set("A1", "=MyRef");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "5");

    model._set("B1", "99"); // edit the name's target
    model.evaluate();
    assert_eq!(model._get_text("A1"), "99");
}

#[test]
fn incremental_defined_name_retarget_forces_full() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    let sheet = model.workbook.worksheets[0].get_name();
    model._set("A1", "10");
    model._set("C1", "777");
    model
        .new_defined_name("myname", None, &format!("{sheet}!$A$1"))
        .unwrap();
    model._set("B1", "=myname");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "10");

    model._set("Z1", "1"); // arms incremental with a pending edit
    model
        .update_defined_name("myname", None, "myname", None, &format!("{sheet}!$C$1"))
        .unwrap(); // reparses and evaluates; must fall back to full
    assert_eq!(model._get_text("B1"), "777"); // not the stale 10
}

#[test]
fn incremental_range_clear_contents_forces_full() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=A1");
    model._set("C1", "=B1");
    model._set("D1", "0");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "1");

    model._set("D1", "5"); // pending edit arms incremental
    model
        .range_clear_contents(&crate::expressions::types::Area {
            sheet: 0,
            row: 1,
            column: 2,
            width: 1,
            height: 1,
        })
        .unwrap(); // clears B1; must fall back to full
    model.evaluate();
    assert_eq!(model._get_text("C1"), "0"); // B1 cleared, not the stale 1
}

#[test]
fn incremental_set_array_formula_forces_full() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("D1", "10");
    model._set("D2", "=D1+1");
    model.evaluate();

    model._set("D1", "20"); // pending edit arms incremental
    model
        .set_user_array_formula(0, 1, 1, 1, 2, "=B1:B2")
        .unwrap(); // spills A1:A2
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    assert_eq!(model._get_text("A2"), "2"); // spill produced, not #ERROR!
}

#[test]
fn incremental_offset_reads_dynamic_target() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "10");
    model._set("D2", "=C1");
    model._set("A1", "=OFFSET(D1,1,0)"); // reads D2, a target no static edge captures
    model.evaluate();
    assert_eq!(model._get_text("A1"), "10");

    model._set("C1", "20"); // D2 -> 20; A1 must follow, not read a stale D2
    model.evaluate();
    assert_eq!(model._get_text("A1"), "20");
}

#[test]
fn incremental_set_locale_forces_full() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1234.5");
    model._set("B1", "=TEXT(A1,\"#,##0.00\")");
    model._set("Z1", "0"); // independent
    model.evaluate();
    assert_eq!(model._get_text("B1"), "1,234.50");

    model._set("Z1", "1"); // arms incremental with a pending edit
    model.set_locale("de").unwrap(); // evaluates internally; must force full
    assert_eq!(model._get_text("B1"), "1.234,50"); // not the stale en format
}

#[test]
fn incremental_tracks_dynamic_branch_dependencies() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("D1", "1");
    model._set("E1", "10");
    model._set("F1", "20");
    model._set("A1", "=IF(D1>0,E1,F1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "10"); // reads E1

    model._set("D1", "-1"); // flip: A1 now reads F1
    model.evaluate();
    assert_eq!(model._get_text("A1"), "20");

    model._set("F1", "99"); // edit the newly-read cell
    model.evaluate();
    assert_eq!(model._get_text("A1"), "99");
}

#[test]
fn incremental_tracks_function_read_references() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("A2", "2");
    model._set("A3", "3");
    model._set("B1", "=SUBTOTAL(9,A1:A3)");
    model._set("D1", "1"); // selector
    model._set("E1", "10");
    model._set("F1", "20");
    model._set("D2", "=CHOOSE(D1,E1,F1)");
    model.evaluate();

    model._set("A1", "10"); // inside SUBTOTAL's range
    model._set("E1", "99"); // CHOOSE's selected branch
    model.evaluate();
    assert_eq!(model._get_text("B1"), "15"); // 1+2+3 -> 10+2+3
    assert_eq!(model._get_text("D2"), "99"); // CHOOSE -> E1
}

#[test]
fn incremental_tracks_named_lambda_body_references() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    let sheet = model.workbook.worksheets[0].get_name();
    model
        .new_defined_name("TAX", None, &format!("=LAMBDA(amt, amt + {sheet}!$B$1)"))
        .unwrap();
    model._set("B1", "5"); // closed over
    model._set("C1", "100"); // argument
    model._set("A1", "=TAX(C1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "105");

    model._set("B1", "9"); // edit the closed-over cell
    model.evaluate();
    assert_eq!(model._get_text("A1"), "109");
}

#[test]
fn incremental_tracks_volatile_in_lambda_body() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model
        .new_defined_name("VOL", None, "=LAMBDA(a, a + INDIRECT(\"A1\"))")
        .unwrap();
    model._set("A1", "5"); // read dynamically, no static edge
    model._set("B1", "=VOL(10)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "15");

    model._set("A1", "50");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "60");
}

#[test]
fn incremental_tracks_dynamic_range_operator() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "10");
    model._set("A2", "20");
    model._set("A3", "30");
    model._set("C1", "3"); // row count
    model._set("B1", "=SUM(A1:OFFSET(A1,C1-1,0))"); // sums A1 down to row C1
    model.evaluate();
    assert_eq!(model._get_text("B1"), "60"); // 10+20+30

    model._set("A2", "25"); // interior cell
    model.evaluate();
    assert_eq!(model._get_text("B1"), "65");
}

#[test]
fn incremental_undo_under_pause_stays_correct() {
    let mut model = UserModel::new_empty("m", "en", "UTC", "en")
        .unwrap()
        .with_recalc_mode(incremental_mode());
    model.set_user_input(0, 1, 1, "1").unwrap(); // A1
    model.set_user_input(0, 1, 2, "=A1+1").unwrap(); // B1 = 2
    model.set_user_input(0, 1, 3, "10").unwrap(); // C1 (independent)
    model.set_user_input(0, 1, 4, "=C1+1").unwrap(); // D1
    model.set_user_input(0, 1, 1, "5").unwrap(); // A1 = 5 -> B1 = 6

    model.pause_evaluation();
    model.undo().unwrap(); // A1 back to 1, evaluation deferred
    model.set_user_input(0, 1, 3, "20").unwrap(); // unrelated edit re-arms incremental
    model.resume_evaluation();
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 1, 2).unwrap(), "2"); // B1 repaired, not stale 6
    assert_eq!(model.get_formatted_cell_value(0, 1, 4).unwrap(), "21"); // D1
}
