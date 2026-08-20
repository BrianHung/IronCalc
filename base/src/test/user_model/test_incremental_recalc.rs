#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

use crate::test::util::{incremental_mode, new_empty_model};
use crate::types::CellType;
use crate::{ChangedSinceRead, UserModel};

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
    assert!(model.graph.referenced.contains(&(0, 1, 1, 5, 1)));

    // An insert only shifts references, so the graph shifts its edges rather
    // than rebuilding: the next pass stays incremental.
    model.insert_rows(0, 3, 2).unwrap(); // A1:A5 -> A1:A7
    assert!(!model.graph.should_recompute_full());
    assert!(model.graph.referenced.contains(&(0, 1, 1, 7, 1)));
    assert!(!model.graph.referenced.contains(&(0, 1, 1, 5, 1)));
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
    assert!(
        model.graph.volatile.contains(&(0, 1, 2)),
        "SUM((A1):(A10)) nests OpRangeKind; it must be volatile"
    );
    assert_eq!(model._get_text("B1"), "2");

    model._set("A5", "10");
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
fn incremental_sum_over_offset_sees_updated_target() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "10");
    model._set("D2", "=A1");
    model._set("A3", "=OFFSET(D1,1,0)");
    model._set("C1", "=SUM(A1:A3)");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "20");
    let _ = model.take_changed_cells();

    model._set("A1", "20");
    model.evaluate();
    assert_eq!(model._get_text("D2"), "20");
    assert_eq!(model._get_text("A3"), "20");
    assert_eq!(model._get_text("C1"), "40");
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(3, 1))); // A3 OFFSET
    assert!(changed.contains(&(1, 3))); // C1 SUM
}

#[test]
fn incremental_running_totals_compose_after_offset_and_insert() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("A2", "2");
    model._set("A3", "3");
    model._set("B1", "=SUM(A1:A1)");
    model._set("B2", "=SUM(A1:A2)");
    model._set("B3", "=SUM(A1:A3)");
    model._set("D2", "=A2");
    model._set("C1", "=OFFSET(D1,1,0)"); // reads D2
    model.evaluate();
    assert_eq!(model._get_text("B1"), "1");
    assert_eq!(model._get_text("B2"), "3");
    assert_eq!(model._get_text("B3"), "6");
    assert_eq!(model._get_text("C1"), "2");

    model._set("A2", "5");
    model.evaluate();
    assert_eq!(model._get_text("B2"), "6");
    assert_eq!(model._get_text("B3"), "9");
    assert_eq!(model._get_text("C1"), "5");

    model.insert_rows(0, 2, 1).unwrap();
    model._set("A2", "4"); // new row must change every prefix that covers it
    model.evaluate();
    assert_eq!(model._get_formula("B4"), "=SUM(A1:A4)");
    assert_eq!(model._get_text("B1"), "1");
    assert_eq!(model._get_text("B3"), "10"); // 1 + 4 + 5
    assert_eq!(model._get_text("B4"), "13"); // 1 + 4 + 5 + 3
    assert_eq!(model._get_text("C1"), "0"); // still OFFSET(D1,1,0); D2 is the new blank
}

#[test]
fn incremental_running_totals_see_offset_inside_range() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("A2", "2");
    model._set("D2", "3");
    model._set("A3", "=OFFSET(D1,1,0)"); // inside the running-total range
    model._set("B1", "=SUM(A1:A1)");
    model._set("B2", "=SUM(A1:A2)");
    model._set("B3", "=SUM(A1:A3)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "1");
    assert_eq!(model._get_text("B2"), "3");
    assert_eq!(model._get_text("B3"), "6");
    let _ = model.take_changed_cells();

    model._set("D2", "10"); // OFFSET target; A3 and B3 must not read a stale memo
    model.evaluate();
    assert_eq!(model._get_text("A3"), "10");
    assert_eq!(model._get_text("B1"), "1");
    assert_eq!(model._get_text("B2"), "3");
    assert_eq!(model._get_text("B3"), "13");
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(3, 1))); // A3
    assert!(changed.contains(&(3, 2))); // B3
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
    // INDIRECT is stored as a 1×1 dynamic array, so the cone takes the
    // array/spill full path. That is Everything, not an incremental delta.
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);
}

#[test]
fn incremental_offset_does_not_force_full_on_unrelated_edit() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "10");
    model._set("A1", "=OFFSET(C1,0,0)");
    model._set("Z1", "1");
    model.evaluate();
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);
    assert!(model.graph.dynamic_refs.contains(&(0, 1, 1)));
    assert!(model.graph.volatile.contains(&(0, 1, 1)));

    // OFFSET is volatile and a dynamic ref, not an array cell. An unrelated
    // edit must stay incremental instead of falling back to a full workbook pass.
    model._set("Z1", "2");
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
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    assert_eq!(model._get_text("A2"), "2");
    assert_eq!(model._get_text("C11"), "2");
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

    model._set("D1", "=A1+1"); // a new formula forces a full recompute
    model.evaluate();
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);
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
fn range_composition_does_not_memoize_transient_circ() {
    // A2's SUM sees A2 mid-cycle (#CIRC → IFERROR → 5). That must not be
    // cached for B1, which should see A2 after it settles.
    for incremental in [false, true] {
        let mut model = if incremental {
            new_empty_model().with_recalc_mode(incremental_mode())
        } else {
            new_empty_model()
        };
        model._set("A1", "1");
        model._set("A2", "=IFERROR(SUM(A1:A2),5)");
        model._set("B1", "=SUM(A1:A2)");
        model.evaluate();
        assert_eq!(model._get_text("A2"), "5", "incremental={incremental}");
        assert_eq!(model._get_text("B1"), "6", "incremental={incremental}");
    }
}

#[test]
fn full_mode_sum_matches_precomposition_association() {
    let mut model = new_empty_model();
    model.update_cell_with_number(0, 1, 1, 1e16).unwrap();
    model.update_cell_with_number(0, 1, 2, 1.0).unwrap();
    model.update_cell_with_number(0, 2, 1, -1e16).unwrap();
    model.update_cell_with_number(0, 2, 2, 1.0).unwrap();
    model._set("C1", "=SUM(A1:B2)");
    model.evaluate();
    // Row-major 1e16+1-1e16+1 = 1. Per-row composition yields 0.
    assert_eq!(
        model.get_cell_value_by_index(0, 1, 3).unwrap(),
        crate::cell::CellValue::Number(1.0)
    );
}

#[test]
fn range_composition_does_not_memoize_transient_count_circ() {
    for incremental in [false, true] {
        let mut model = if incremental {
            new_empty_model().with_recalc_mode(incremental_mode())
        } else {
            new_empty_model()
        };
        model._set("A1", "1");
        model._set("A2", "=COUNT(A1:A2)");
        model._set("B1", "=COUNT(A1:A2)");
        model.evaluate();
        assert_eq!(model._get_text("A2"), "1", "incremental={incremental}");
        assert_eq!(model._get_text("B1"), "2", "incremental={incremental}");
    }
}
