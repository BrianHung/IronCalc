#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

use crate::recalc::Input;
use crate::test::util::{incremental_mode, new_empty_model};
use crate::types::CellType;
use crate::{ChangedSinceRead, RecalcMode, UserModel};

fn reads_random(model: &crate::Model, p: (u32, i32, i32)) -> bool {
    model.graph.cell_reads(p, |i| matches!(i, Input::Random))
}

fn reads_own_coord(model: &crate::Model, p: (u32, i32, i32)) -> bool {
    model
        .graph
        .cell_reads(p, |i| matches!(i, Input::OwnCoord(_)))
}

fn flush_writes(model: &mut crate::Model) {
    model.drain_write_journal();
}

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
fn incremental_error_is_not_a_same_text_literal() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=1/A1");
    model.evaluate();
    assert_eq!(model._get_cell("B1").get_type(), CellType::Number);

    // Value edit, not a formula write: `=…` would force_full and skip Verify.
    // CellValue stores errors as text, so type is the only Error vs "#DIV/0!" distinction.
    model._set("A1", "0");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_cell("B1").get_type(), CellType::ErrorValue);
    assert_eq!(model._get_text("B1"), "#DIV/0!");
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
    assert!(reads_random(&model, (0, 1, 2)));
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
    flush_writes(&mut model);
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
    flush_writes(&mut model);
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
    flush_writes(&mut model);
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
    assert!(reads_random(&model, (0, 10, 1)));

    // The graph shifts every position-keyed set, so the Random input travels
    // to A11 and the next pass can stay incremental.
    model.insert_rows(0, 1, 1).unwrap();
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    assert!(reads_random(&model, (0, 11, 1)));
    assert!(!reads_random(&model, (0, 10, 1)));

    model.evaluate();
    assert!(reads_random(&model, (0, 11, 1)));
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
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("C11"), "2");
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
fn incremental_nested_dynamic_range_tracks_interior() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("A5", "0");
    model._set("A10", "1");
    // Parentheses keep the colon as OpRangeKind under SUM. Static edges are
    // only the endpoints; A5 is missed unless the SUM is marked volatile.
    model._set("B1", "=SUM((A1):(A10))");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "2");

    model._set("A5", "10");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("B1"), "12");
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
    // B1 was cleared; C1 reads a formula blank, which coerces to 0
    // (Excel parity) — the assertion is that it is not the stale 1.
    assert_eq!(model._get_text("C1"), "0");
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
    model._set("B1", "=A1"); // static dependent of the dynamic ref
    model.evaluate();
    assert_eq!(model._get_text("A1"), "10");
    assert_eq!(model._get_text("B1"), "10");

    model._set("C1", "20"); // D2 -> 20; A1 must follow, not read a stale D2
    model.evaluate();
    assert_eq!(model._get_text("A1"), "20");
    assert_eq!(model._get_text("B1"), "20");
}

#[test]
fn incremental_chained_offset_reads_updated_target() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "10");
    model._set("E2", "=C1");
    model._set("D2", "=OFFSET(E1,1,0)"); // dynamic ref whose target is E2
    model._set("A1", "=OFFSET(D1,1,0)"); // dynamic ref whose target is D2
    model.evaluate();
    assert_eq!(model._get_text("A1"), "10");

    // No static edge A1→D2. If D2 is still Evaluated, A1 reads 10.
    model._set("C1", "20");
    model.evaluate();
    assert_eq!(model._get_text("D2"), "20");
    assert_eq!(model._get_text("A1"), "20");
}

#[test]
fn incremental_offset_through_helper_reads_updated_target() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "10");
    model._set("F2", "=C1");
    model._set("E2", "=OFFSET(F1,1,0)"); // reads F2
    model._set("D2", "=E2"); // static helper — in the cone, not a seed
    model._set("A1", "=OFFSET(D1,1,0)"); // reads D2; no edge D2→A1
    model.evaluate();
    assert_eq!(model._get_text("A1"), "10");
    let _ = model.take_changed_cells();

    model._set("C1", "20");
    model.evaluate();
    assert_eq!(model._get_text("E2"), "20");
    assert_eq!(model._get_text("D2"), "20");
    assert_eq!(model._get_text("A1"), "20");
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(2, 4))); // D2 helper
    assert!(changed.contains(&(1, 1))); // A1 OFFSET
}

#[test]
fn incremental_indirect_through_helper_reads_updated_target() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "10");
    model._set("F2", "=C1");
    model._set("E2", "=INDIRECT(\"F2\")");
    model._set("D2", "=E2");
    model._set("A1", "=INDIRECT(\"D2\")");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "10");
    let _ = model.take_changed_cells();

    model._set("C1", "20");
    model.evaluate();
    assert_eq!(model._get_text("E2"), "20");
    assert_eq!(model._get_text("D2"), "20");
    assert_eq!(model._get_text("A1"), "20");
    // INDIRECT is stored as a 1×1 dynamic array; its last result was 1×1, so
    // the pass stays incremental and the delta is the precise chain
    // C1 → F2 → E2 → D2 → A1, traced through both INDIRECT hops.
    match model.take_changed_cells() {
        ChangedSinceRead::Cells(cells) => assert_eq!(cells.len(), 5),
        other => panic!("expected a precise delta, got {other:?}"),
    }
}

#[test]
fn incremental_offset_does_not_force_full_on_unrelated_edit() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "10");
    model._set("A1", "=OFFSET(C1,0,0)");
    model._set("Z1", "1");
    model.evaluate();
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);

    // OFFSET's actual target is a traced edge, not a role set. An unrelated
    // edit must stay incremental instead of falling back to a full workbook pass.
    model._set("Z1", "2");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("OFFSET must not force a workbook-wide full pass");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 26))); // Z1
    assert!(!changed.contains(&(1, 1))); // A1 OFFSET ran; value did not move
}

#[test]
fn incremental_hyperlink_over_offset_keeps_link_on_unrelated_edit() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "http://a.com");
    model._set("A1", "=OFFSET(C1,0,0)");
    model._set("B1", "=HYPERLINK(A1,\"click\")");
    model._set("Z1", "1");
    model.evaluate();
    let dynamic_at = |model: &crate::Model| {
        model
            .get_links_list(0)
            .unwrap()
            .into_iter()
            .filter(|l| l.dynamic)
            .map(|l| (l.row, l.column))
            .collect::<Vec<_>>()
    };
    assert_eq!(dynamic_at(&model), vec![(1, 2)]); // B1

    model._set("Z1", "2"); // A1 unchanged; B1 must not lose its URL
    model.evaluate();
    assert_eq!(model._get_text("B1"), "click");
    assert_eq!(dynamic_at(&model), vec![(1, 2)]);
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
fn incremental_reports_locale_change() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1234.5");
    model._set("B1", "=TEXT(A1,\"#,##0.00\")");
    model._set("Z1", "0"); // independent
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("Z1", "1"); // a pending edit that alone would be a small delta
    model.set_locale("de").unwrap();
    // A locale change re-renders every formatted value, so no small delta
    // describes it; the record reports everything changed.
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);
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
fn incremental_insert_below_dynamic_array_rebuilds_spill() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("A1", "=B1:B2"); // spills A1:A2
    model._set("Z1", "0");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    assert_eq!(model._get_text("A2"), "2");

    model._set("Z1", "1"); // dirty an unrelated cell
    model.insert_rows(0, 6, 1).unwrap();
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    assert_eq!(model._get_text("A2"), "2"); // spill left intact, not #ERROR!
}

#[test]
fn incremental_insert_below_dynamic_array_and_volatile_stays_incremental() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("A1", "=B1:B2"); // spills A1:A2, entirely above the insert
    model._set("C10", "=1+1");
    model.evaluate();

    model.insert_rows(0, 5, 1).unwrap();
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    assert_eq!(model._get_text("A2"), "2");
    assert_eq!(model._get_text("C11"), "2");
}

/// A blocked spill (`#SPILL!`) stores `r=(1,1)`. An insert below it still
/// resets the anchor so the next pass can re-try the spill. That reset must
/// journal even though the formula index is unchanged (fuzz seed 51).
#[test]
fn blocked_spill_reset_on_insert_below_matches_full() {
    let mut inc = new_empty_model().with_recalc_mode(incremental_mode());
    inc._set("A1", "1");
    inc._set("A2", "2");
    inc._set("A3", "3");
    inc._set("A4", "4");
    inc._set("B1", "4");
    inc._set("B2", "3");
    inc._set("B3", "2");
    inc._set("B4", "1");
    inc._set("E17", "=SORTBY(A1:A4,B1:B4)");
    inc._set("E18", "=FILTER(A1:A4,B1:B4>0)");
    inc.evaluate();
    inc.insert_rows(0, 21, 1).unwrap();
    inc.evaluate();

    let mut full = new_empty_model();
    full._set("A1", "1");
    full._set("A2", "2");
    full._set("A3", "3");
    full._set("A4", "4");
    full._set("B1", "4");
    full._set("B2", "3");
    full._set("B3", "2");
    full._set("B4", "1");
    full._set("E17", "=SORTBY(A1:A4,B1:B4)");
    full._set("E18", "=FILTER(A1:A4,B1:B4>0)");
    full.evaluate();
    full.insert_rows(0, 21, 1).unwrap();
    full.evaluate();

    assert_eq!(inc._get_text("E17"), full._get_text("E17"));
    assert_eq!(
        inc._get_cell("E17").get_type(),
        full._get_cell("E17").get_type()
    );
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

#[test]
fn take_changed_cells_reports_incremental_delta() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=A1*2");
    model._set("C1", "100"); // independent
    model.evaluate();
    // Switching mode forces a full pass, so everything is potentially changed.
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);

    model._set("A1", "5"); // A1 and its dependent B1 recompute; C1 untouched
    model.evaluate();
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 1))); // A1 edited
    assert!(changed.contains(&(1, 2))); // B1 recomputed
    assert!(!changed.contains(&(1, 3))); // C1 not touched

    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Cells(vec![])); // reading clears

    model._set("D1", "=A1+1"); // a new formula records its edges on first evaluate
    model.evaluate();
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("a new scalar formula must stay Incremental");
    };
    assert!(cells.iter().any(|c| c.row == 1 && c.column == 4));
}

#[test]
fn take_changed_cells_reports_structural_edit_delta() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "10");
    model._set("B10", "=A1+1"); // depends on A1, value 11
    model.evaluate();
    let _ = model.take_changed_cells(); // clear the mode-switch full

    // Delete row 1: A1 is removed so B10's reference dangles, and B10 shifts up
    // to B9. Data cells in the shift band are not in the dirty cone, so the
    // delta cannot name every moved cell. Report Everything.
    model.delete_rows(0, 1, 1).unwrap();
    model.evaluate();

    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);
    assert_eq!(model._get_text("B9"), "#REF!");
}

#[test]
fn take_changed_cells_reports_everything_for_data_only_shift() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "10");
    model._set("A2", "20");
    model._set("Z1", "1");
    model.evaluate();
    let _ = model.take_changed_cells();

    // Insert above two data cells. Nothing is a formula dependent, so a cell
    // list would be empty while A1/A2 visibly moved.
    model.insert_rows(0, 1, 1).unwrap();
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);
    assert_eq!(model._get_text("A2"), "10");
    assert_eq!(model._get_text("A3"), "20");

    // The flag dies with the pass. A later value edit is a cell list again.
    model._set("Z1", "2");
    model.evaluate();
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("structural Everything must not leak into the next pass");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 26)));
}

#[test]
fn take_changed_cells_reports_everything_for_trailing_delete() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "10");
    model.evaluate();
    let _ = model.take_changed_cells();

    // Last populated row, nothing below, no formulas. Dirty stays empty so
    // evaluate would take the Full keep-delta branch; the emptied cell must
    // still be Everything, not Cells([]).
    model.delete_rows(0, 1, 1).unwrap();
    // Ready + empty dirty: the Full keep-delta branch, not MustRebuild.
    assert!(model.graph.should_recompute_full());
    assert!(!model.graph.full_reflects_change());
    model.evaluate();
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);
    assert_eq!(model._get_text("A1"), "");
}

#[test]
fn take_changed_cells_survives_redundant_evaluate() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=A1*2");
    model.evaluate();
    let _ = model.take_changed_cells(); // clear the mode-switch full

    model._set("A1", "5");
    model.evaluate(); // incremental: records A1, B1
    model.evaluate(); // redundant no-op full: must keep the delta

    // The delta survives the redundant evaluate, so this is Cells, not Everything.
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 1)));
    assert!(changed.contains(&(1, 2)));
}

#[test]
fn take_changed_cells_survives_redundant_evaluate_with_offset() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "10");
    model._set("A1", "=OFFSET(C1,0,0)");
    model._set("B1", "=C1+1");
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("C1", "20");
    model.evaluate();
    model.evaluate(); // OFFSET does not re-roll; must keep the incremental delta
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("OFFSET must not wipe the delta to Everything");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 3))); // C1
    assert!(changed.contains(&(1, 2))); // B1
}

#[test]
fn phase_two_restores_memo_for_skipped_cone_cells() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("E2", "5");
    model._set("D2", "=E2");
    model._set("A1", "=OFFSET(D2,0,0)");
    model.evaluate();
    model._set("Z1", "1");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert!(
        model.cells.contains_key(&(0, 2, 4)),
        "D2 must stay Evaluated after phase 2 skips it"
    );
    assert_eq!(model._get_text("A1"), "5");
    assert_eq!(model._get_text("D2"), "5");
}

#[test]
fn redundant_evaluate_with_volatile_reports_full_change() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=A1+1");
    model._set("C1", "=RAND()"); // volatile: re-rolls on every full pass
    model.evaluate();
    let _ = model.take_changed_cells(); // clear the mode-switch full

    model._set("A1", "5");
    model.evaluate(); // incremental edit
    let _ = model.take_changed_cells(); // clear that delta

    model.evaluate(); // redundant full: re-rolls C1, so not a no-op
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything); // not Cells([])
}

#[test]
fn incremental_reports_dynamic_link_retarget() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "http://a.com");
    model._set("B1", "=HYPERLINK(A1,\"click\")"); // label "click", target A1
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("A1", "http://b.com"); // label unchanged, only the target moves
    model.evaluate();
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 2))); // B1's link changed, so it is reported
}

#[test]
fn incremental_reports_conditional_format_change() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "5");
    model._set("B1", "x");
    model
        .add_conditional_formatting(
            0,
            "B1",
            crate::cf_types::CfRuleInput::Formula {
                formula: "=$A$1>10".to_string(),
                format: crate::types::Dxf::default(),
                stop_if_true: false,
            },
        )
        .unwrap();
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("A1", "20"); // flips B1's rule on, though B1's value is unchanged
    model.evaluate();
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 2))); // B1's conditional format changed
}

#[test]
fn incremental_reports_cf_only_mutation() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "5");
    model._set("B1", "x");
    model.evaluate();
    let _ = model.take_changed_cells();

    model
        .add_conditional_formatting(
            0,
            "B1",
            crate::cf_types::CfRuleInput::Formula {
                formula: "=$A$1>3".to_string(),
                format: crate::types::Dxf::default(),
                stop_if_true: false,
            },
        )
        .unwrap();
    model.evaluate(); // a CF-rule edit with no cell value change
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 2))); // B1's new format is reported
}

#[test]
fn incremental_reports_signed_zero_flip() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "5");
    model._set("B1", "=A1*0");
    model._set("C1", "=B1&\"!\"");
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("A1", "-5"); // B1: +0 -> -0, observable in C1's text
    model.evaluate();
    assert_eq!(model._get_text("C1"), "-0!"); // not the stale "0!"
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 3))); // C1 reported
}

#[test]
fn incremental_reports_only_value_changes() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=A1*0"); // always 0
    model._set("C1", "=B1+1"); // always 1
    model.evaluate();
    let _ = model.take_changed_cells(); // clear the mode-switch full

    model._set("A1", "5");
    model.evaluate();
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 1))); // A1 moved 1 -> 5
    assert!(!changed.contains(&(1, 2))); // B1 recomputed but stayed 0
    assert!(!changed.contains(&(1, 3))); // C1 not reached
    assert_eq!(model._get_text("C1"), "1"); // still correct
}

#[test]
fn incremental_propagates_error_to_text_transition() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1"); // truthy: take the error branch
    model._set("B1", "=IF(A1, 1/0, \"#DIV/0!\")"); // error #DIV/0!
    model._set("C1", "=ISERROR(B1)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "#DIV/0!");
    assert_eq!(model._get_text("C1"), "TRUE");

    let _ = model.take_changed_cells();
    model._set("A1", "0"); // falsy: B1 becomes the literal text "#DIV/0!"
    model.evaluate();
    assert_eq!(model._get_text("B1"), "#DIV/0!"); // same string, now text
    assert_eq!(model._get_text("C1"), "FALSE"); // dependent must flip
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 2))); // B1 type-only
    assert!(changed.contains(&(1, 3))); // C1 dependent
}

#[test]
fn incremental_sumifs_reads_resized_criteria() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("A2", "1");
    model._set("A3", "1");
    model._set("B1", "1");
    model._set("B2", "1");
    model._set("B3", "1");
    model._set("C1", "=SUMIFS(A1:A3,B1:B1,\">0\")");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "3");

    model._set("B2", "0");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("C1"), "2");
}

#[test]
fn incremental_cell_name_survives_insert() {
    // Verify's interleaved full pass rebuilds the graph and hides this class.
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    let sheet = model.workbook.worksheets[0].get_name();
    model._set("A10", "42");
    model
        .new_defined_name("MyRef", None, &format!("{sheet}!$A$10"))
        .unwrap();
    model._set("B1", "=MyRef");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "42");

    // Insert between the formula and the name target so B1 stays put while
    // the old A10 shifts to A11. The name still reads $A$10.
    model.insert_rows(0, 5, 1).unwrap();
    model.evaluate();
    // The name now reads a blank cell: a blank formula result coerces to 0
    // (Excel parity), not "".
    assert_eq!(model._get_text("B1"), "0");

    model._set("A10", "99");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("B1"), "99");
}

#[test]
fn incremental_argless_row_updates_after_insert() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=ROW()");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    assert!(reads_own_coord(&model, (0, 1, 1)));

    model.insert_rows(0, 1, 1).unwrap();
    model.evaluate();
    assert_eq!(model._get_text("A2"), "2");
}

/// The semantic behind the journal pause in `write_displaced_formula`: the raw
/// write runs with the journal off (a displacement is not a formula edit) and
/// the function substitutes a value-write entry for it. If the pause swallowed
/// the write instead of standing in for it, FORMULATEXT would keep reporting
/// the pre-displacement text. This behaviour is the point;
/// `JournalRecordingPaused` is only the mechanism.
#[test]
fn incremental_formulatext_sees_displaced_formula() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B1", "1");
    model._set("A1", "=SUM(B1)");
    model._set("C1", "=FORMULATEXT(A1)");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "=SUM(B1)");

    model.insert_rows(0, 1, 1).unwrap();
    model.evaluate();
    assert_eq!(model._get_text("C2"), "=SUM(B2)");
}

#[test]
fn incremental_subtotal_sees_hidden_row() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("A2", "2");
    model._set("A3", "3");
    model._set("B1", "=SUBTOTAL(109,A1:A3)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "6");

    model.set_row_hidden(0, 2, true).unwrap();
    model._set("D9", "1");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("B1"), "4");
}

#[test]
fn incremental_overwrite_rand_clears_volatile() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=RAND()");
    model.evaluate();
    assert!(reads_random(&model, (0, 1, 1)));

    model._set("A1", "5");
    model.evaluate();
    assert!(!reads_random(&model, (0, 1, 1)));
    assert_eq!(model._get_text("A1"), "5");

    model._set("B1", "1");
    model.evaluate();
    model._set("B1", "2");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "5");
    assert!(!reads_random(&model, (0, 1, 1)));
}

#[test]
fn incremental_update_cell_apis_clear_volatile_roles() {
    for overwrite in [
        |model: &mut crate::Model| model.update_cell_with_number(0, 1, 1, 5.0).unwrap(),
        |model: &mut crate::Model| model.update_cell_with_text(0, 1, 1, "5").unwrap(),
        |model: &mut crate::Model| model.update_cell_with_bool(0, 1, 1, true).unwrap(),
    ] {
        let mut model = new_empty_model().with_recalc_mode(incremental_mode());
        model._set("A1", "=RAND()");
        model.evaluate();
        assert!(reads_random(&model, (0, 1, 1)));

        overwrite(&mut model);
        model.evaluate();
        assert!(!reads_random(&model, (0, 1, 1)));
        let _ = model.take_changed_cells();

        model._set("B1", "1");
        model.evaluate();
        model._set("B1", "2");
        model.evaluate();
        assert!(!reads_random(&model, (0, 1, 1)));

        let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
            panic!("expected incremental delta");
        };
        let changed: std::collections::HashSet<(i32, i32)> =
            cells.iter().map(|c| (c.row, c.column)).collect();
        assert!(
            !changed.contains(&(1, 1)),
            "overwritten RAND must not stay in later deltas"
        );
    }
}

#[test]
fn incremental_overwrite_spill_anchor_updates_dependents() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=SEQUENCE(3)");
    model._set("C2", "=A2*10");
    model.evaluate();
    assert_eq!(model._get_text("A2"), "2");
    assert_eq!(model._get_text("C2"), "20");

    // Keep the anchor in `arrays` so this pass Full-falls-back; clearing that
    // role would stay Incremental and leave C2 stale after the spill is wiped.
    model._set("A1", "5");
    assert!(model.graph.arrays.contains(&(0, 1, 1)));
    model.evaluate();
    assert_eq!(model._get_text("A1"), "5");
    assert_eq!(model._get_text("A2"), "");
    assert_eq!(model._get_text("A3"), "");
    assert_eq!(model._get_text("C2"), "0");
    assert!(!model.graph.arrays.contains(&(0, 1, 1)));
}

#[test]
fn incremental_rejected_write_does_not_poison_evaluate() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model.evaluate();
    assert!(model.set_user_input(1, 9, 3, "84".to_string()).is_err());
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
}

#[test]
fn incremental_blocked_spill_respills_after_insert() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("E3", "7");
    model._set("E1", "=SEQUENCE(3)");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "#SPILL!");

    model.insert_rows(0, 2, 2).unwrap();
    model.evaluate();
    assert_eq!(model._get_text("E1"), "1");
    assert_eq!(model._get_text("E2"), "2");
    assert_eq!(model._get_text("E3"), "3");
}

#[test]
fn incremental_empty_passthrough_counts_as_blank() {
    // A formula that returns blank coerces to 0 (Excel parity), so D1 holds a
    // value and COUNTBLANK(D1:E1) is 0: neither the passthrough D1 nor the
    // text in E1 counts. Pinned in both modes, across an unrelated edit to E1
    // that keeps the second pass incremental.
    let run = |mode: crate::RecalcMode| {
        let mut model = new_empty_model().with_recalc_mode(mode);
        model._set("C1", "=COUNTBLANK(D1:E1)");
        model._set("D1", "=A1");
        model._set("E1", "x");
        model.evaluate();
        let first = model._get_text("C1");
        model._set("E1", "y");
        model.evaluate();
        (first, model._get_text("C1"))
    };
    let full = run(crate::RecalcMode::Full);
    assert_eq!(full, ("0".to_string(), "0".to_string()));
    assert_eq!(full, run(crate::RecalcMode::Incremental));
}

#[test]
fn incremental_empty_passthrough_concat() {
    // A blank formula result coerces to 0 at the result boundary (Excel parity),
    // so `D1&G1` with D1 blank renders "01"/"02" — see the D1 table: A2&"x"
    // is "0x", matching main and Excel.
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B1", "=D1&G1");
    model._set("D1", "=IF(TRUE,A1,1)");
    model._set("G1", "1");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "01");

    model._set("G1", "2");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "02");
}

#[test]
fn incremental_sumifs_index_criteria_tracks_expanded_range() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    for row in 1..=10 {
        model.set_user_input(0, row, 1, "1".to_string()).unwrap();
        model.set_user_input(0, row, 4, "1".to_string()).unwrap();
    }
    model._set("E1", "=SUMIFS(D1:D10,INDEX(A1:C3,0,1),\">0\")");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "10");

    model._set("A7", "0");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "9");
}

#[test]
fn incremental_sumifs_let_criteria_tracks_expanded_range() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    for row in 1..=10 {
        model.set_user_input(0, row, 1, "1".to_string()).unwrap();
        model.set_user_input(0, row, 4, "1".to_string()).unwrap();
    }
    model._set("E1", "=LET(r,A1:A3,SUMIFS(D1:D10,r,\">0\"))");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "10");

    model._set("A7", "0");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "9");
}

#[test]
fn incremental_offset_cone_delta_only_net_movers() {
    for _ in 0..40 {
        let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
        model._set("C1", "0");
        model._set("D1", "10");
        model._set("E1", "=C1*10");
        model._set("A1", "=OFFSET(D1,0,C1)");
        model._set("G1", "=A1+1");
        model.evaluate();
        let _ = model.take_changed_cells();
        model._set("C1", "1");
        model.evaluate();
        assert_eq!(model._get_text("A1"), "10");
        match model.take_changed_cells() {
            ChangedSinceRead::Everything => panic!("expected a cells delta"),
            ChangedSinceRead::Cells(cells) => {
                assert!(
                    !cells
                        .iter()
                        .any(|c| c.row == 1 && (c.column == 1 || c.column == 7)),
                    "unchanged A1/G1 in delta: {cells:?}"
                );
            }
        }
    }
}

#[test]
fn incremental_redundant_evaluate_reports_late_spill() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    model._set("E15", "=SEQUENCE(3)");
    model._set("A1", "=E15#");
    model.evaluate();
    let _ = model.take_changed_cells();
    model.evaluate();
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => {}
        ChangedSinceRead::Cells(cells) => {
            if model._get_text("A2") == "2" {
                assert!(
                    cells.iter().any(|c| c.row == 2 && c.column == 1),
                    "A2 spilled on the redundant pass but is missing from the delta: {cells:?}"
                );
            }
        }
    }
}

#[test]
fn incremental_blocked_spill_sum_agrees_with_full() {
    // A blocked dynamic array must not skip the arrays→Full guard, and SUM of
    // the spill column must see #SPILL! rather than a stale number.
    fn run(mode: crate::RecalcMode) -> (String, String) {
        let mut model = new_empty_model().with_recalc_mode(mode);
        model._set("G1", "=SUM(E:E)");
        model._set("E8", "=SUM(B3:C9)");
        model._set("E19", "=A1:A3*2");
        model._set("E20", "=OFFSET(A1,1,1)");
        model.evaluate();
        model._set("C4", "5");
        model.evaluate();
        (model._get_text("E19"), model._get_text("G1"))
    }
    assert_eq!(
        run(crate::RecalcMode::Full),
        run(crate::RecalcMode::Incremental)
    );
}

#[test]
fn stored_empty_formula_is_live_zero() {
    // Excel coerces a blank formula result to 0 at the result boundary
    // (cached `<v>0</v>`), and the store must agree with what same-pass and
    // out-of-cone readers see, in every mode.
    let run = |mode: crate::RecalcMode| {
        let mut model = new_empty_model().with_recalc_mode(mode);
        model._set("B1", "=A1");
        model.evaluate();
        (
            model._get_text("B1"),
            model.get_cell_value_by_index(0, 1, 2).unwrap(),
        )
    };
    assert_eq!(run(crate::RecalcMode::Full).0, "0");
    assert_eq!(
        run(crate::RecalcMode::Incremental),
        run(crate::RecalcMode::Full)
    );
}

#[test]
fn offset_target_change_without_static_edge() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    model._set("C1", "0");
    model._set("D1", "1");
    model._set("E1", "2");
    model._set("A1", "=OFFSET(D1,0,C1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    let _ = model.take_changed_cells();

    model._set("E1", "99");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("unrelated E1 edit must stay Incremental"),
        ChangedSinceRead::Cells(cells) => {
            let changed: std::collections::HashSet<(i32, i32)> =
                cells.iter().map(|c| (c.row, c.column)).collect();
            assert!(changed.contains(&(1, 5)));
            assert!(!changed.contains(&(1, 1)));
        }
    }

    model._set("C1", "1");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "99");
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("OFFSET retarget must stay Incremental"),
        ChangedSinceRead::Cells(cells) => {
            let changed: std::collections::HashSet<(i32, i32)> =
                cells.iter().map(|c| (c.row, c.column)).collect();
            assert!(changed.contains(&(1, 1)));
        }
    }
}

#[test]
fn sumifs_criteria_from_index_is_tracked() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    for row in 1..=10 {
        model.set_user_input(0, row, 1, "1".to_string()).unwrap();
        model.set_user_input(0, row, 4, "1".to_string()).unwrap();
    }
    model._set("E1", "=SUMIFS(D1:D10,INDEX(A1:C3,0,1),\">0\")");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "10");
    let _ = model.take_changed_cells();

    model._set("A7", "0");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "9");
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("SUMIFS INDEX criteria must stay Incremental"),
        ChangedSinceRead::Cells(_) => {}
    }
}

#[test]
fn subtotal_sees_hidden_row_it_scans_not_own_row() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    for row in 1..=10 {
        model._set(&format!("A{row}"), "1");
    }
    model._set("A50", "=SUBTOTAL(109,A1:A10)");
    model.evaluate();
    assert_eq!(model._get_text("A50"), "10");
    let _ = model.take_changed_cells();

    model.set_row_hidden(0, 5, true).unwrap();
    model.evaluate();
    assert_eq!(model._get_text("A50"), "9");
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("hide scanned row must stay Incremental"),
        ChangedSinceRead::Cells(cells) => {
            assert!(cells.iter().any(|c| c.row == 50 && c.column == 1));
        }
    }

    model.set_row_hidden(0, 50, true).unwrap();
    model.evaluate();
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("hide own row must not force Full"),
        ChangedSinceRead::Cells(cells) => {
            assert!(
                !cells.iter().any(|c| c.row == 50 && c.column == 1),
                "SUBTOTAL must not recompute when its own row is hidden"
            );
        }
    }
    assert_eq!(model._get_text("A50"), "9");
}

#[test]
fn name_reader_redirty_on_insert() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    let sheet = model.workbook.worksheets[0].get_name();
    model._set("A10", "42");
    model
        .new_defined_name("MyRef", None, &format!("{sheet}!$A$10"))
        .unwrap();
    model._set("B1", "=MyRef");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "42");

    model.insert_rows(0, 5, 1).unwrap();
    model.evaluate();
    // The name now reads a blank cell: a blank formula result coerces to 0
    // (Excel parity), not "".
    assert_eq!(model._get_text("B1"), "0");

    model._set("A10", "99");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "99");
}

#[test]
fn name_reader_redirty_on_insert_sheet_scoped() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    let sheet = model.workbook.worksheets[0].get_name();
    model._set("A10", "42");
    model
        .new_defined_name("LocalRef", Some(0), &format!("{sheet}!$A$10"))
        .unwrap();
    model._set("B1", "=LocalRef");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "42");

    model.insert_rows(0, 5, 1).unwrap();
    model.evaluate();
    // The name now reads a blank cell: a blank formula result coerces to 0
    // (Excel parity), not "".
    assert_eq!(model._get_text("B1"), "0");

    model._set("A10", "99");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "99");
}

#[test]
fn redundant_evaluate_keeps_rand_reporting_but_not_sumifs() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    model._set("A1", "1");
    model._set("B1", "=SUMIFS(A1:A1,A1:A1,\">0\")");
    model._set("C1", "=RAND()");
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("A1", "5");
    model.evaluate();
    let _ = model.take_changed_cells();

    model.evaluate();
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => {}
        other => panic!("RAND must keep reporting Everything, got {other:?}"),
    }

    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    model._set("A1", "1");
    model._set("B1", "=SUMIFS(A1:A1,A1:A1,\">0\")");
    model.evaluate();
    let _ = model.take_changed_cells();
    model._set("A1", "5");
    model.evaluate();
    let _ = model.take_changed_cells();
    model.evaluate();
    match model.take_changed_cells() {
        ChangedSinceRead::Cells(cells) => assert!(
            cells.is_empty(),
            "SUMIFS must not always-report on a redundant evaluate"
        ),
        ChangedSinceRead::Everything => {
            panic!("SUMIFS must not wipe the delta on a redundant evaluate")
        }
    }
}

/// SUBTOTAL records FormulaText of scanned cells. A formula write in that
/// range re-evals SUBTOTAL but must not always-report it when the aggregate
/// is unchanged (fuzz seed 31).
#[test]
fn subtotal_formula_text_reread_is_not_always_reported() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    model._set("A1", "1");
    model._set("B1", "=SUBTOTAL(9,A1:A5)");
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("A2", "=0");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "1");
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("SUBTOTAL reread must stay Incremental"),
        ChangedSinceRead::Cells(cells) => {
            let changed: std::collections::HashSet<(i32, i32)> =
                cells.iter().map(|c| (c.row, c.column)).collect();
            assert!(
                changed.contains(&(2, 1)),
                "A2 is a write seed, got {changed:?}"
            );
            assert!(
                !changed.contains(&(1, 2)),
                "SUBTOTAL must not always-report when the aggregate is unchanged, got {changed:?}"
            );
        }
    }
}

#[test]
fn formula_edit_stays_incremental() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    model._set("A1", "1");
    model._set("B1", "=A1+1");
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("B1", "=A1+2");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("B1"), "3");
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("formula edit must stay Incremental"),
        ChangedSinceRead::Cells(_) => {}
    }

    model._set("A1", "5");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "7");
}

#[test]
fn journal_value_over_formula_drops_edges() {
    for overwrite in [
        |model: &mut crate::Model| model.update_cell_with_number(0, 1, 1, 5.0).unwrap(),
        |model: &mut crate::Model| model.update_cell_with_text(0, 1, 1, "5").unwrap(),
        |model: &mut crate::Model| model.update_cell_with_bool(0, 1, 1, true).unwrap(),
        |model: &mut crate::Model| {
            model.set_user_input(0, 1, 1, "5".to_string()).unwrap();
        },
    ] {
        let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
        model._set("A1", "=RAND()");
        model.evaluate();
        overwrite(&mut model);
        model.evaluate();
        let _ = model.take_changed_cells();
        model._set("B1", "1");
        model.evaluate();
        model._set("B1", "2");
        model.evaluate();
        match model.take_changed_cells() {
            ChangedSinceRead::Everything => panic!("overwritten RAND must not force Everything"),
            ChangedSinceRead::Cells(cells) => {
                assert!(
                    !cells.iter().any(|c| c.row == 1 && c.column == 1),
                    "overwritten RAND must not stay in later deltas"
                );
            }
        }
    }
}

#[test]
fn formulatext_sees_value_overwrite_of_its_argument() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=1+1");
    model._set("B1", "=FORMULATEXT(A1)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "=1+1");

    model._set("A1", "ov");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("B1"), "#N/A");
}

#[test]
fn isformula_sees_value_overwrite_of_its_argument() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=1+1");
    model._set("B1", "=ISFORMULA(A1)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "TRUE");

    model.update_cell_with_number(0, 1, 1, 36.0).unwrap();
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("B1"), "FALSE");
}

/// SEQUENCE + a `#` spill-ref can take a second Full pass to materialize
/// spill cells. An unrelated write on another sheet must not leave those
/// cells missing on Incremental (fuzz seed 52).
#[test]
fn sequence_hash_spill_survives_unrelated_other_sheet_edit() {
    let mut inc = new_empty_model().with_recalc_mode(incremental_mode());
    inc.add_sheet("Data").unwrap();
    inc.set_user_input(0, 15, 5, "=SEQUENCE(3)".to_string())
        .unwrap();
    inc.set_user_input(0, 13, 7, "=E15#".to_string()).unwrap();
    inc.evaluate();
    let g14_after_first = inc._get_text("G14");

    let mut full = new_empty_model();
    full.add_sheet("Data").unwrap();
    full.set_user_input(0, 15, 5, "=SEQUENCE(3)".to_string())
        .unwrap();
    full.set_user_input(0, 13, 7, "=E15#".to_string()).unwrap();
    full.evaluate();

    inc.set_user_input(1, 4, 1, "38".to_string()).unwrap();
    inc.evaluate();
    full.set_user_input(1, 4, 1, "38".to_string()).unwrap();
    full.evaluate();

    assert_eq!(
        inc._get_text("G14"),
        full._get_text("G14"),
        "after first evaluate Incremental G14={g14_after_first:?}"
    );
}

#[test]
fn style_on_blank_cell_does_not_enter_the_delta() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model.evaluate();
    let _ = model.take_changed_cells();
    let mut style = model.get_style_for_cell(0, 13, 3).unwrap();
    style.fill.color = crate::types::Color::Rgb("#FFAA00".to_string());
    model.set_cell_style(0, 13, 3, &style).unwrap();
    model.evaluate();
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("style on a blank cell must stay Incremental"),
        ChangedSinceRead::Cells(cells) => assert!(
            cells.is_empty(),
            "style on a blank cell must not appear in the delta, got {cells:?}"
        ),
    }
}

#[test]
fn journal_rejected_write_logs_nothing() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    model._set("A1", "1");
    model.evaluate();
    let _ = model.take_changed_cells();
    assert!(model.set_user_input(1, 9, 3, "84".to_string()).is_err());
    model.evaluate();
    match model.take_changed_cells() {
        ChangedSinceRead::Cells(cells) => assert!(cells.is_empty()),
        ChangedSinceRead::Everything => panic!("rejected write must not force Everything"),
    }
}

/// `E15#` of an empty cell is `#REF!` until E15 is written. The `#` operator
/// must record a cell edge on the empty anchor so the later write re-runs it
/// (fuzz seed 3, 200 steps).
#[test]
fn spill_hash_of_empty_anchor_sees_later_formula() {
    let mut inc = new_empty_model().with_recalc_mode(incremental_mode());
    inc._set("E16", "=E15#");
    inc.evaluate();
    inc._set("E15", "=ROWS(A1:A8)");
    inc.evaluate();

    let mut full = new_empty_model();
    full._set("E16", "=E15#");
    full.evaluate();
    full._set("E15", "=ROWS(A1:A8)");
    full.evaluate();

    assert_eq!(inc._get_text("E16"), full._get_text("E16"));
    assert_eq!(inc._get_text("E16"), "8");
}

/// `COUNT(F15#)` of an empty F15 is 0 until F15 is written (fuzz seed 32).
#[test]
fn count_of_empty_spill_hash_sees_later_formula() {
    let mut inc = new_empty_model().with_recalc_mode(incremental_mode());
    inc._set("G4", "=COUNT(F15#)");
    inc.evaluate();
    inc._set("F15", "=SUM(A1:C6)");
    inc.evaluate();

    let mut full = new_empty_model();
    full._set("G4", "=COUNT(F15#)");
    full.evaluate();
    full._set("F15", "=SUM(A1:C6)");
    full.evaluate();

    assert_eq!(inc._get_text("G4"), full._get_text("G4"));
}

/// `=E15#` above `=SEQUENCE(3)`: Full's first pass leaves E11 empty (the `#`
/// ref is still a CellFormula, not in spill_cells). Incremental must not be a
/// second pass on already-promoted arrays (fuzz seed 31).
#[test]
fn sequence_hash_above_anchor_matches_full_on_first_eval() {
    let mut inc = new_empty_model().with_recalc_mode(incremental_mode());
    inc.new_defined_name("LAM", None, "LAMBDA(x,x*2+Sheet1!$A$2)")
        .unwrap();
    inc._set("E15", "=SEQUENCE(3)");
    inc._set("E10", "=E15#");
    inc.evaluate();

    let mut full = new_empty_model();
    full.new_defined_name("LAM", None, "LAMBDA(x,x*2+Sheet1!$A$2)")
        .unwrap();
    full._set("E15", "=SEQUENCE(3)");
    full._set("E10", "=E15#");
    full.evaluate();

    for cell in ["E10", "E11", "E12", "E15", "E16", "E17"] {
        assert_eq!(inc._get_text(cell), full._get_text(cell), "{cell}");
    }
}

#[test]
fn undo_redo_under_incremental_stays_incremental() {
    let mut model = UserModel::new_empty("m", "en", "UTC", "en")
        .unwrap()
        .with_recalc_mode(incremental_mode());
    model.set_user_input(0, 1, 1, "1").unwrap();
    model.set_user_input(0, 1, 2, "=A1+1").unwrap();
    model.evaluate();
    let _ = model.model.take_changed_cells();
    model.set_user_input(0, 1, 1, "5").unwrap();
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 1, 2).unwrap(), "6");
    model.undo().unwrap();
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 1, 2).unwrap(), "2");
    match model.model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("undo of a value edit must stay Incremental"),
        ChangedSinceRead::Cells(_) => {}
    }
}

/// The column move rebuilds the moved column one cell at a time, in row order
/// (`column_cell_references` sorts), so the anchor of a CSE array is written
/// back before the placeholders of the rectangle it re-declares. That is an
/// interim state of the move, not a user write: `move_column_unchecked` must
/// suspend the CSE member guard around the rebuild (the way `move_cell` does)
/// or the placeholder writes error and the move fails. The mode does not
/// matter -- the rebuild happens before any recalc -- so Full alone is pinned.
#[test]
fn displace_rounds_numeric_constants_to_excel_precision() {
    let mut model = new_empty_model();
    model._set("A2", "0");
    model._set("B2", "=A2+0.30000000000000004");
    model.insert_rows(0, 1, 1).unwrap();
    model.evaluate();
    let mut expected = new_empty_model();
    expected._set("A3", "0");
    expected._set("B3", "=A3+0.30000000000000004");
    expected.evaluate();
    assert_eq!(
        model.get_cell_value_by_index(0, 3, 2).unwrap(),
        expected.get_cell_value_by_index(0, 3, 2).unwrap()
    );
}

#[test]
fn displace_strips_quote_prefix_from_formulas() {
    let mut model = new_empty_model();
    model._set("B2", "=A2+1");
    let style = model.get_cell_style_index(0, 2, 2).unwrap();
    let quoted = model
        .workbook
        .styles
        .get_style_with_quote_prefix(style)
        .unwrap();
    model
        .workbook
        .worksheet_mut(0)
        .unwrap()
        .set_cell_style(2, 2, quoted)
        .unwrap();
    assert!(model
        .workbook
        .styles
        .style_is_quote_prefix(model.get_cell_style_index(0, 2, 2).unwrap()));
    model.insert_rows(0, 1, 1).unwrap();
    assert!(!model
        .workbook
        .styles
        .style_is_quote_prefix(model.get_cell_style_index(0, 3, 2).unwrap()));
}

#[test]
fn moving_a_column_with_a_cse_anchor_always_succeeds() {
    let mut model = new_empty_model().with_recalc_mode(RecalcMode::Full);
    model
        .set_user_array_formula(0, 1, 5, 1, 2, "=B1:B2")
        .unwrap();
    model.move_columns_action(0, 5, 1, 2).unwrap();
    model.evaluate();
    assert_eq!(model._get_formula("G1"), "=B1:B2");
}

/// The same rebuild along the other axis: `move_row_unchecked` writes the
/// anchor and then the placeholders of a horizontal rectangle, in column
/// order, and must suspend the guard the same way.
#[test]
fn moving_a_row_with_a_cse_anchor_always_succeeds() {
    let mut model = new_empty_model().with_recalc_mode(RecalcMode::Full);
    model
        .set_user_array_formula(0, 5, 1, 2, 1, "=A1:B1")
        .unwrap();
    model.move_rows_action(0, 5, 1, 2).unwrap();
    model.evaluate();
    assert_eq!(model._get_formula("A7"), "=A1:B1");
}

/// A read of a multi-column rectangle must be recorded as a rectangle, not
/// dropped. `SUM(B:C)` clips its per-cell walk to the used range, so the only
/// edge that can connect a write below the last used row to the sum is the
/// recorded rect. Dropping wide rects from the read set leaves A1 stale.
#[test]
fn multi_column_range_edits_propagate() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B1", "1");
    model._set("A1", "=SUM(B:C)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");

    // Second column of the rect, past the used range: only the rectangle
    // connects this write to A1.
    model._set("C100", "5");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("A1"), "6");

    // ... and the first column of the same rect.
    model._set("B50", "4");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "10");
}

/// The whole-row twin: `SUM(2:3)` is a two-row rect spanning every column, and
/// the per-cell walk stops at the used column. A write to the right of it must
/// still re-fire the sum.
#[test]
fn multi_column_whole_row_range_edits_propagate() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B2", "1");
    model._set("A1", "=SUM(2:3)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");

    model._set("Z3", "5");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("A1"), "6");

    // A write outside the two rows must not be mistaken for one inside them.
    model._set("Z4", "1000");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "6");
}

/// A volatile cell must re-roll on every incremental pass, not only on a full
/// one: it stays in the always-dirty set, which seeds every cone and every
/// delta. Unrelated value edits keep the passes incremental; RANDBETWEEN has
/// to move anyway, its dependent has to follow, and the delta has to name it
/// on every pass. Several passes, so one unlucky equal draw cannot flake.
#[test]
fn volatile_rerolls_across_repeated_incremental_passes() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=RANDBETWEEN(1,1000000000)");
    model._set("A2", "=A1+0"); // a dependent: the re-roll must propagate
    model._set("B2", "0");
    model.evaluate();
    let _ = model.take_changed_cells();

    let first = model.get_cell_value_by_index(0, 1, 1).unwrap();
    let first_dependent = model.get_cell_value_by_index(0, 2, 1).unwrap();
    let mut rerolled = false;
    let mut dependent_rerolled = false;
    for pass in 1..=5 {
        model._set("B2", &pass.to_string());
        flush_writes(&mut model);
        assert!(!model.graph.should_recompute_full());
        model.evaluate();
        if model.get_cell_value_by_index(0, 1, 1).unwrap() != first {
            rerolled = true;
        }
        if model.get_cell_value_by_index(0, 2, 1).unwrap() != first_dependent {
            dependent_rerolled = true;
        }
        let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
            panic!("an unrelated value edit must stay incremental");
        };
        assert!(
            cells
                .iter()
                .any(|c| (c.sheet, c.row, c.column) == (0, 1, 1)),
            "the delta must name the volatile cell on every pass: {cells:?}"
        );
    }
    assert!(rerolled, "RANDBETWEEN never re-rolled over five passes");
    assert!(
        dependent_rerolled,
        "the volatile's dependent never followed the re-roll"
    );
}

/// NOW() is volatile too, and the clock is mocked in the test build, so its
/// value cannot be used to detect a re-roll. What is observable is that it
/// stays in the always-dirty set and is reported on every incremental pass.
#[test]
fn clock_volatile_is_reported_on_every_incremental_pass() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=NOW()");
    model._set("A2", "=TODAY()");
    model._set("B2", "0");
    model.evaluate();
    let _ = model.take_changed_cells();

    for pass in 1..=3 {
        model._set("B2", &pass.to_string());
        flush_writes(&mut model);
        assert!(!model.graph.should_recompute_full());
        model.evaluate();
        let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
            panic!("an unrelated value edit must stay incremental");
        };
        for position in [(0, 1, 1), (0, 2, 1)] {
            assert!(
                cells.iter().any(|c| (c.sheet, c.row, c.column) == position),
                "the delta must name the clock volatile {position:?}: {cells:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Convergence: incremental must equal full after *every* evaluate, including
// the passes where full heals debt its own previous pass left behind. Each
// test below drives both modes in lockstep over a minimized shape from the
// differential fuzzer and compares the whole workbook after each evaluate.
// ---------------------------------------------------------------------------

/// Every populated cell's value, for a mode-vs-mode comparison.
fn workbook_values(model: &crate::Model) -> Vec<(u32, i32, i32, String)> {
    let mut cells: Vec<(u32, i32, i32, String)> = model
        .get_all_cells()
        .into_iter()
        .map(|c| {
            (
                c.index,
                c.row,
                c.column,
                model
                    .get_formatted_cell_value(c.index, c.row, c.column)
                    .unwrap_or_default(),
            )
        })
        .collect();
    cells.sort();
    cells
}

fn assert_same_workbook(full: &crate::Model, incremental: &crate::Model, label: &str) {
    assert_eq!(
        workbook_values(full),
        workbook_values(incremental),
        "incremental diverged from full after {label}"
    );
}

/// (a) A row move forces a full pass, but that pass leaves spill debt: `G12`
/// read `E16:E17` before `E15`'s `SEQUENCE` refilled them. Full heals on its
/// next unconditional pass; incremental used to serve `G12` its stored 0 for
/// ever, because no later cone ever named it.
#[test]
fn incremental_heals_spill_debt_left_by_a_forced_full_pass() {
    let build = |mode| {
        let mut m = new_empty_model().with_recalc_mode(mode);
        m._set("E15", "=SEQUENCE(3)");
        m._set("G12", "=SUM(E19:E20)");
        m._set("F14", "=SEQUENCE(3)+G12");
        m
    };
    let mut full = build(crate::RecalcMode::Full);
    let mut inc = build(incremental_mode());
    for m in [&mut full, &mut inc] {
        m.evaluate();
        m.move_rows_action(0, 19, 2, -3).unwrap();
        m.evaluate();
    }
    assert_same_workbook(&full, &inc, "the move's own full pass");
    // The healing pass. The edit is deliberately unrelated to the spill: the
    // dirty cone cannot reach G12, so only the debt signal brings it back.
    for m in [&mut full, &mut inc] {
        m._set("B5", "1");
        m.evaluate();
    }
    assert_eq!(full._get_text("G12"), "5");
    assert_same_workbook(&full, &inc, "the healing pass");
}

/// (b) A newly set formula closes a cycle, here entirely through range reads.
/// The closing edge is only observed while the pass runs, so the cone was
/// ordered without it and `#CIRC!` landed on a different member than full's
/// recursion picks; the pass must redo itself as full instead.
#[test]
fn incremental_places_circ_like_full_when_an_edit_closes_a_cycle() {
    let build = |mode| {
        let mut m = new_empty_model().with_recalc_mode(mode);
        m._set("F7", "=AVERAGE(A1:B10)");
        m._set("C10", "=SUM(F4,F7)");
        m
    };
    let mut full = build(crate::RecalcMode::Full);
    let mut inc = build(incremental_mode());
    for m in [&mut full, &mut inc] {
        m.evaluate();
        // A7 is inside A1:B10, so F7 now reads a cell that reads F7's column.
        m._set("A7", "=COUNTBLANK(A1:D12)");
        m.evaluate();
    }
    assert_eq!(full._get_text("F7"), "#CIRC!");
    assert_same_workbook(&full, &inc, "the cycle-closing pass");
}

/// (b) An error-absorbing function makes the divergence a value, not just a
/// placement: the frontier evaluated `B1` once through `A1`'s recursion (the
/// mid-cycle value full keeps) and then a second time at `B1`'s own topological
/// slot, against the settled `A1`. Full evaluates each cell once per pass.
#[test]
fn incremental_does_not_re_evaluate_a_mid_cycle_cell() {
    let build = |mode| {
        let mut m = new_empty_model().with_recalc_mode(mode);
        m._set("A1", "1");
        m._set("B1", "=IFERROR(A1+1,50)");
        m
    };
    let mut full = build(crate::RecalcMode::Full);
    let mut inc = build(incremental_mode());
    for m in [&mut full, &mut inc] {
        m.evaluate();
        m._set("A1", "=B1");
        m.evaluate();
    }
    assert_eq!(full._get_text("A1"), "50");
    assert_eq!(full._get_text("B1"), "50");
    assert_same_workbook(&full, &inc, "the IFERROR cycle pass");
}

/// (b) The same second-evaluation divergence with no formula edit at all: a
/// value edit flips an `IF` branch, and the branch it turns on closes the
/// cycle. Nothing in the graph predicts the new read.
#[test]
fn incremental_does_not_re_evaluate_a_mid_cycle_cell_reached_by_a_branch() {
    let build = |mode| {
        let mut m = new_empty_model().with_recalc_mode(mode);
        m._set("D1", "-1");
        m._set("A1", "=IF(D1>0,IFERROR(B1,7),5)");
        m._set("B1", "=A1+1");
        m
    };
    let mut full = build(crate::RecalcMode::Full);
    let mut inc = build(incremental_mode());
    for m in [&mut full, &mut inc] {
        m.evaluate();
        m._set("D1", "1");
        m.evaluate();
    }
    assert_eq!(full._get_text("A1"), "7");
    assert_eq!(full._get_text("B1"), "#CIRC!");
    assert_same_workbook(&full, &inc, "the branch-closed cycle pass");
}

/// (c) A CSE anchor rebuilt by column moves, whose refilled rectangle is read
/// by a formula the anchor itself reads. The full pass resolves that cycle
/// against the members' stored values, so the refill leaves the reader holding
/// the old ones; full heals on its next unconditional pass, and incremental
/// used to keep the stale pair for ever.
#[test]
fn incremental_heals_cse_refill_debt() {
    let build = |mode| {
        let mut m = new_empty_model().with_recalc_mode(mode);
        m.set_user_array_formula(0, 1, 3, 1, 3, "=A1:A3+1").unwrap();
        m
    };
    let mut full = build(crate::RecalcMode::Full);
    let mut inc = build(incremental_mode());
    for m in [&mut full, &mut inc] {
        m.evaluate();
        // B3 reads A2:C2, which contains the anchor's own member C2.
        m._set("A3", "=SUM(B2:C2)");
        m.move_columns_action(0, 1, 1, 1).unwrap();
        m.evaluate();
    }
    assert_same_workbook(&full, &inc, "the move's own full pass");
    // Full's own value moves on this pass with nothing relevant edited: that is
    // the debt the previous pass left, and incremental has to move with it.
    assert_eq!(full._get_text("B3"), "1");
    for m in [&mut full, &mut inc] {
        m._set("B4", "3");
        m.evaluate();
    }
    assert_eq!(full._get_text("B3"), "0");
    assert_same_workbook(&full, &inc, "the CSE healing pass");
}

/// Plain edits with no arrays, no structural ops and no cycles must not pay for
/// any of the above: the pass stays selective and reports a cell delta.
#[test]
fn convergence_guard_leaves_plain_edits_selective() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    model._set("A1", "1");
    model._set("B1", "=A1+1");
    model._set("C1", "=B1+1");
    model.evaluate();
    for value in ["2", "3", "4"] {
        let _ = model.take_changed_cells();
        model._set("A1", value);
        model.evaluate();
        match model.take_changed_cells() {
            ChangedSinceRead::Everything => panic!("a plain edit fell back to a full pass"),
            ChangedSinceRead::Cells(cells) => assert!(!cells.is_empty()),
        }
    }
    assert_eq!(model._get_text("C1"), "6");
}

/// A cycle closed through a dynamic-array anchor whose result is 1x1. The
/// anchor spills nothing, so it is not in `graph.arrays`; what routes the pass
/// to Full is that a cycle member is a seed on every pass and this seed is an
/// anchor. It has to: Full evaluates anchors in a phase of their own, ahead of
/// the row-major walk, so `#CIRC!` lands inside `B2`'s recursion and `A1`
/// absorbs it -- and the cone walk `recompute_all` would otherwise do is
/// row-major only, entering at `A1` and settling the cycle on the other side,
/// for ever. The plain-formula run pins that the anchor's phase is what decides
/// the side: with a plain formula in `B2` the two runs would agree, and the
/// inequality below fails before any real divergence can.
#[test]
fn recompute_all_places_circ_through_a_scalar_anchor_like_full() {
    let run = |mode, b2: &str| {
        let mut m = new_empty_model().with_recalc_mode(mode);
        m._set("D1", "1");
        m._set("A1", "=IFERROR(B2,100)+1+D1*0");
        m._set("B2", b2);
        m.evaluate();
        let mut states = vec![(m._get_text("A1"), m._get_text("B2"))];
        // Plain value edits: no structural op, no new formula, and the cycle
        // is already an edge, so each pass stays selective and reaches
        // `recompute_all`. Snapshot after every single pass -- a later full
        // fallback can heal a one-pass divergence, so only the per-pass view
        // sees it -- then once more after a redundant evaluate, which used to
        // be the state nothing ever brought back.
        for value in ["2", "3"] {
            m._set("D1", value);
            m.evaluate();
            states.push((m._get_text("A1"), m._get_text("B2")));
        }
        m.evaluate();
        states.push((m._get_text("A1"), m._get_text("B2")));
        states
    };
    let anchor = "=SEQUENCE(1,1,IFERROR(A1,200)+1,1)";
    let plain = "=IFERROR(A1,200)+1";
    let full_anchor = run(crate::RecalcMode::Full, anchor);
    // Full's placement, stable on every pass: B2 evaluates first (phase 1),
    // its recursion into A1 hits the cycle inside A1's IFERROR, so A1
    // absorbs it.
    assert_eq!(full_anchor, vec![("101".to_string(), "102".to_string()); 4]);
    assert_eq!(full_anchor, run(incremental_mode(), anchor));
    assert_eq!(
        run(crate::RecalcMode::Full, plain),
        run(incremental_mode(), plain)
    );
    assert_ne!(
        full_anchor,
        run(crate::RecalcMode::Full, plain),
        "the anchor no longer evaluates ahead of the rest of the cone"
    );
}

/// The CSE variant of the same shape. A CSE anchor is in `graph.arrays` whatever
/// its size, so this one leaves through the arrays→Full fallback rather than
/// through `recompute_all`'s ordering; either way the placement has to match.
#[test]
fn incremental_places_circ_like_full_through_a_cse_anchor() {
    let build = |mode| {
        let mut m = new_empty_model().with_recalc_mode(mode);
        m._set("D1", "1");
        m._set("A1", "=IFERROR(B2,100)+1+D1*0");
        m.set_user_array_formula(0, 2, 2, 1, 1, "=IFERROR(A1,200)+1")
            .unwrap();
        m
    };
    let mut full = build(crate::RecalcMode::Full);
    let mut inc = build(incremental_mode());
    for m in [&mut full, &mut inc] {
        m.evaluate();
    }
    assert_same_workbook(&full, &inc, "the first evaluate");
    for m in [&mut full, &mut inc] {
        m._set("D1", "2");
        m.evaluate();
    }
    assert_same_workbook(&full, &inc, "the known-cycle pass through the CSE anchor");
    for m in [&mut full, &mut inc] {
        m._set("D1", "3");
        m.evaluate();
        m.evaluate();
    }
    assert_same_workbook(&full, &inc, "the following passes");
}

/// A blocked anchor stores `#SPILL!`, but a reader that reaches it while it is
/// still evaluating gets the live array's top-left instead -- here `B1`, an
/// anchor of full's phase 1, pulls `A7` in ahead of `E15`, so full's `A7` holds
/// `1 + D1` and not `-1 + D1`. That value is not a function of the store: a
/// later cone that names `A7` without naming `E15` would recompute it against
/// the stored error and land on `-1 + D1` for ever. Only the full pass
/// evaluates the anchor live, so a cone reaching a blocked anchor's reader has
/// to fall back to one.
#[test]
fn a_blocked_anchors_reader_is_recomputed_only_by_a_full_pass() {
    let build = |mode| {
        let mut m = new_empty_model().with_recalc_mode(mode);
        m._set("E15", "=SEQUENCE(3)");
        // E17 blocks the spill, so E15 stores #SPILL!.
        m._set("E17", "7");
        m._set("D1", "1");
        m._set("A7", "=IFERROR(E15,-1)+D1");
        // A phase-1 anchor above A7: full evaluates it, and so A7, before it
        // reaches E15 in its own right.
        m._set("B1", "=SEQUENCE(1,1,A7,1)");
        m
    };
    let mut full = build(crate::RecalcMode::Full);
    let mut inc = build(incremental_mode());
    for m in [&mut full, &mut inc] {
        m.evaluate();
    }
    assert_eq!(full._get_text("E15"), "#SPILL!");
    assert_eq!(full._get_text("A7"), "2");
    assert_same_workbook(&full, &inc, "the first evaluate");
    // A plain value edit whose cone names A7 but not E15.
    for m in [&mut full, &mut inc] {
        m._set("D1", "2");
        m.evaluate();
    }
    assert_eq!(full._get_text("A7"), "3");
    assert_same_workbook(&full, &inc, "an edit reaching a blocked anchor's reader");
}

/// The audit of the acyclic path: `recompute_frontier` orders by edges, not by
/// position, so a scalar anchor's readers are already ordered after it and the
/// phase-1 gap cannot bite -- there is no cycle for a walk order to break, and
/// a dependency-respecting order is unique in what it produces. Here the anchor
/// sits *below* its reader in row-major order, so a one-phase positional walk
/// would be wrong and the topological one is right.
#[test]
fn acyclic_cone_orders_a_scalar_anchor_by_edges_not_position() {
    let build = |mode| {
        let mut m = new_empty_model().with_recalc_mode(mode);
        m._set("D1", "1");
        // A1 (walked first, row-major) reads B9, the anchor (walked last).
        m._set("A1", "=B9+1");
        m._set("B9", "=SEQUENCE(1,1,D1*10,1)");
        m
    };
    let mut full = build(crate::RecalcMode::Full);
    let mut inc = build(incremental_mode());
    for m in [&mut full, &mut inc] {
        m.evaluate();
    }
    assert_same_workbook(&full, &inc, "the first evaluate");
    for value in ["2", "3"] {
        for m in [&mut full, &mut inc] {
            m._set("D1", value);
            m.evaluate();
        }
        assert_same_workbook(&full, &inc, "an acyclic edit reaching a scalar anchor");
    }
    assert_eq!(full._get_text("A1"), "31");
}

/// Verify's liveness check asserts against the always-dirty set as it stood at
/// pass start, because that is the set `evaluate_selective` seeds
/// `always_report` from. A cell whose branch flips *into* `RAND()` records the
/// input only while it evaluates, so it was never seeded; with `RAND()*0` its
/// value does not move either, and the delta rightly leaves it out. Asserting
/// against the post-pass set panicked on exactly this.
#[cfg(feature = "recalc_verify")]
#[test]
fn verify_liveness_allows_a_cell_that_becomes_volatile_mid_pass() {
    let mut model = new_empty_model().with_recalc_mode(RecalcMode::Verify);
    model._set("D1", "-1");
    model._set("A1", "=IF(D1>0,RAND()*0,0)");
    model.evaluate();
    assert!(!reads_random(&model, (0, 1, 1)));
    model._set("D1", "1");
    model.evaluate();
    assert!(reads_random(&model, (0, 1, 1)));
    assert_eq!(model._get_text("A1"), "0");
    // Steady state: from here A1 is in the pre-pass set on every pass, so the
    // assertion binds and each pass has to report it.
    for value in ["2", "3"] {
        model._set("D1", value);
        model.evaluate();
        assert!(reads_random(&model, (0, 1, 1)));
    }
}

/// The reverse transition keeps the assertion strong. `A1` is volatile entering
/// the pass, so it is in the pre-pass set, seeds `always_report`, and has to be
/// in the delta -- even though `RAND()*0` means its value never moved and the
/// post-pass set no longer contains it. Asserting against the post-pass set
/// would let a pass drop a volatile cell from its delta unnoticed.
#[cfg(feature = "recalc_verify")]
#[test]
fn verify_liveness_still_binds_when_a_cell_leaves_volatility() {
    let mut model = new_empty_model().with_recalc_mode(RecalcMode::Verify);
    model._set("D1", "1");
    model._set("A1", "=IF(D1>0,RAND()*0,0)");
    model.evaluate();
    assert!(reads_random(&model, (0, 1, 1)));
    let _ = model.take_changed_cells();
    model._set("D1", "-1");
    model.evaluate();
    assert!(!reads_random(&model, (0, 1, 1)));
    assert_eq!(model._get_text("A1"), "0");
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => {}
        ChangedSinceRead::Cells(cells) => assert!(
            cells
                .iter()
                .any(|c| (c.sheet, c.row, c.column) == (0, 1, 1)),
            "the pass that dropped A1's volatility did not report it"
        ),
    }
}

/// The other ways a volatile leaves the always-dirty set: overwritten by a
/// non-volatile formula, and cleared outright. (Overwritten by a plain value
/// is pinned by `incremental_overwrite_rand_clears_volatile` and the
/// update-cell API test, which run under Verify too.) Each removal happens in
/// the write journal, before the next pass snapshots the set, so a liveness
/// assertion holding later passes to a stale entry would demand a report for
/// a cell that no longer re-runs; the selective passes after each teardown
/// are where that would panic.
#[cfg(feature = "recalc_verify")]
#[test]
fn verify_liveness_survives_overwrite_and_clear_of_a_volatile() {
    // Overwritten by a non-volatile formula.
    let mut model = new_empty_model().with_recalc_mode(RecalcMode::Verify);
    model._set("A1", "=RANDBETWEEN(1,1)");
    model.evaluate();
    model._set("A1", "=1+1");
    model.evaluate();
    assert!(!reads_random(&model, (0, 1, 1)));
    assert_eq!(model._get_text("A1"), "2");
    for value in ["1", "2"] {
        model._set("C3", value);
        model.evaluate();
    }

    // Cleared outright.
    let mut model = new_empty_model().with_recalc_mode(RecalcMode::Verify);
    model._set("A1", "=NOW()*0");
    model.evaluate();
    model
        .range_clear_contents(&crate::expressions::types::Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 1,
            height: 1,
        })
        .unwrap();
    model.evaluate();
    assert_eq!(model._get_text("A1"), "");
    for value in ["1", "2"] {
        model._set("C3", value);
        model.evaluate();
    }
}
