//! Seeds 5/126/53 regression: a write into a CSE member position must be
//! rejected, including a member position whose cell a structural delete
//! dropped; the anchor still owns the rectangle and would silently refill it
//! on the next evaluation, destroying the user's input.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use ironcalc_base::{Model, RecalcMode};

#[test]
fn writes_into_cse_members_are_rejected() {
    for mode in [RecalcMode::Full, RecalcMode::Incremental] {
        let mut x = Model::new_empty("t", "en", "UTC", "en")
            .unwrap()
            .with_recalc_mode(mode);
        x.set_user_array_formula(0, 4, 8, 1, 2, "=A1:A3+1").unwrap();
        x.evaluate();
        // H5 is a member of the CSE array anchored at H4.
        assert!(x.set_user_input(0, 5, 8, "99".to_string()).is_err());
        // Delete a column: the array moves to G4 and the member cell at G5 is
        // dropped, but the position is still owned by the anchor's rectangle.
        x.delete_columns(0, 7, 1).unwrap();
        assert!(
            x.set_user_input(0, 5, 7, "99".to_string()).is_err(),
            "write into a displaced CSE member was accepted"
        );
        x.evaluate();
        assert_eq!(
            format!("{:?}", x.get_cell_value_by_index(0, 5, 7).unwrap()),
            "Number(1.0)"
        );
    }
}
