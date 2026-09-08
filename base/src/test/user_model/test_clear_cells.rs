#![allow(clippy::unwrap_used)]

use crate::{expressions::types::Area, test::user_model::util::new_empty_user_model};

#[test]
fn basic() {
    let mut model = new_empty_user_model();
    model.set_user_input(0, 1, 1, "100$").unwrap();
    model
        .range_clear_contents(&Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 1,
            height: 1,
        })
        .unwrap();
    assert_eq!(model.get_formatted_cell_value(0, 1, 1), Ok("".to_string()));
    model.undo().unwrap();
    assert_eq!(
        model.get_formatted_cell_value(0, 1, 1),
        Ok("100$".to_string())
    );
    model.redo().unwrap();
    assert_eq!(model.get_formatted_cell_value(0, 1, 1), Ok("".to_string()));

    model.set_user_input(0, 1, 1, "300").unwrap();
    // clear contents keeps the formatting
    assert_eq!(
        model.get_formatted_cell_value(0, 1, 1),
        Ok("300$".to_string())
    );

    model
        .range_clear_all(&Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 1,
            height: 1,
        })
        .unwrap();
    assert_eq!(model.get_formatted_cell_value(0, 1, 1), Ok("".to_string()));
    model.undo().unwrap();
    assert_eq!(
        model.get_formatted_cell_value(0, 1, 1),
        Ok("300$".to_string())
    );
    model.redo().unwrap();
    assert_eq!(model.get_formatted_cell_value(0, 1, 1), Ok("".to_string()));
    model.set_user_input(0, 1, 1, "400").unwrap();
    // clear contents keeps the formatting
    assert_eq!(
        model.get_formatted_cell_value(0, 1, 1),
        Ok("400".to_string())
    );
}

#[test]
fn clear_empty_cell() {
    let mut model = new_empty_user_model();
    model
        .range_clear_contents(&Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 1,
            height: 1,
        })
        .unwrap();
    assert_eq!(model.get_formatted_cell_value(0, 1, 1), Ok("".to_string()));
    model.undo().unwrap();
    assert_eq!(model.get_formatted_cell_value(0, 1, 1), Ok("".to_string()));
}

#[test]
fn clear_all_empty_cell() {
    let mut model = new_empty_user_model();
    model
        .range_clear_all(&Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 1,
            height: 1,
        })
        .unwrap();
    assert_eq!(model.get_formatted_cell_value(0, 1, 1), Ok("".to_string()));
    model.undo().unwrap();
    assert_eq!(model.get_formatted_cell_value(0, 1, 1), Ok("".to_string()));
}

#[test]
fn issue_454() {
    let mut model = new_empty_user_model();
    model
        .set_user_input(
            0,
            1,
            1,
            "Le presbytère n'a rien perdu de son charme, ni le jardin de son éclat.",
        )
        .unwrap();
    model.set_user_input(0, 1, 2, "=ISTEXT(A1)").unwrap();
    assert_eq!(
        model.get_formatted_cell_value(0, 1, 2),
        Ok("TRUE".to_string())
    );
    model
        .range_clear_contents(&Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 1,
            height: 1,
        })
        .unwrap();
    assert_eq!(
        model.get_formatted_cell_value(0, 1, 2),
        Ok("FALSE".to_string())
    );
    model.undo().unwrap();
}

#[test]
fn issue_454b() {
    let mut model = new_empty_user_model();
    model
        .set_user_input(
            0,
            1,
            1,
            "Le presbytère n'a rien perdu de son charme, ni le jardin de son éclat.",
        )
        .unwrap();
    model.set_user_input(0, 1, 2, "=ISTEXT(A1)").unwrap();
    assert_eq!(
        model.get_formatted_cell_value(0, 1, 2),
        Ok("TRUE".to_string())
    );
    model
        .range_clear_all(&Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 1,
            height: 1,
        })
        .unwrap();
    assert_eq!(
        model.get_formatted_cell_value(0, 1, 2),
        Ok("FALSE".to_string())
    );
    model.undo().unwrap();
}

// range_clear_all over part of a dynamic-array spill tears down the whole
// spill, but only the cells inside the cleared range were selected: a spill
// member inside the range must lose its style with its value, and a member
// outside the range is only reached by the spill teardown and must keep its
// style. A rewrite that tears the footprint down with a plain removal (or
// that skips the in-range sweep for spill members) breaks one side or the
// other. The structural pin is `range_clear_all_spill_teardown_preserves_style`
// in test_recalc_invariants.rs; this is the behavioral one.
#[test]
fn clear_all_over_part_of_a_spill_drops_only_the_selected_styles() {
    let mut model = new_empty_user_model();
    model.set_user_input(0, 1, 1, "3").unwrap();
    // B1 spills over B1:B3
    model.set_user_input(0, 1, 2, "=SEQUENCE(A1)").unwrap();
    assert_eq!(model.get_formatted_cell_value(0, 3, 2), Ok("3".to_string()));
    let spill = Area {
        sheet: 0,
        row: 1,
        column: 2,
        width: 1,
        height: 3,
    };
    model.update_range_style(&spill, "font.b", "true").unwrap();

    // Clear B1:B2: anchor and first member in range, B3 outside it.
    model
        .range_clear_all(&Area {
            sheet: 0,
            row: 1,
            column: 2,
            width: 1,
            height: 2,
        })
        .unwrap();

    for row in 1..=3 {
        assert_eq!(model.get_formatted_cell_value(0, row, 2), Ok(String::new()));
    }
    assert!(!model.get_cell_style(0, 1, 2).unwrap().font.b);
    assert!(
        !model.get_cell_style(0, 2, 2).unwrap().font.b,
        "a spill member inside the cleared range must lose its style"
    );
    assert!(
        model.get_cell_style(0, 3, 2).unwrap().font.b,
        "a spill member outside the cleared range must keep its style"
    );
}
