//! D3 regression: after `delete_columns` drops a CSE member, the anchor still
//! owns its declared rectangle and refills it on a full pass. A reader of the
//! now-ghost member position must therefore not go incremental and serve a
//! stale blank — the arrays guard must cover the declared rectangle.

use ironcalc_base::{Model, RecalcMode};

#[test]
fn cse_ghost_member_reader_matches_full_after_column_delete() {
    // The §8b D3 repro shape: a CSE whose member is dropped by a column delete,
    // then a new formula reading through where the member used to be.
    let run = |mode: RecalcMode| -> String {
        let model = Model::new_empty("m", "en", "UTC", "en").unwrap();
        let mut model = model.with_recalc_mode(mode);
        model.set_user_input(0, 2, 1, "1".to_string()).unwrap();
        model
            .set_user_array_formula(0, 4, 8, 1, 2, "=A1:A3+1")
            .unwrap();
        model.evaluate();
        assert_eq!(
            model.get_formatted_cell_value(0, 4, 8).unwrap(),
            "1",
            "anchor value before the delete"
        );
        model.delete_columns(0, 7, 1).unwrap();
        model
            .set_user_input(0, 5, 7, "=IFERROR(G6,-1)".to_string())
            .unwrap();
        model.evaluate();
        model.get_formatted_cell_value(0, 5, 7).unwrap()
    };
    assert_eq!(run(RecalcMode::Full), run(RecalcMode::Incremental));
}

#[test]
fn cse_member_values_survive_column_delete_next_pass() {
    let run = |mode: RecalcMode| -> Vec<String> {
        let model = Model::new_empty("m", "en", "UTC", "en").unwrap();
        let mut model = model.with_recalc_mode(mode);
        model.set_user_array_formula(0, 1, 1, 2, 2, "=42").unwrap();
        model.evaluate();
        for r in 1..=2 {
            for c in 1..=2 {
                assert_eq!(model.get_formatted_cell_value(0, r, c).unwrap(), "42");
            }
        }
        model.delete_columns(0, 3, 1).unwrap();
        model
            .set_user_input(0, 5, 5, "=A2+100".to_string())
            .unwrap();
        model.evaluate();
        (1..=2)
            .map(|r| model.get_formatted_cell_value(0, r, 1).unwrap())
            .collect()
    };
    assert_eq!(run(RecalcMode::Full), run(RecalcMode::Incremental));
}
