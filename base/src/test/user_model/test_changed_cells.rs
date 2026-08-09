#![allow(clippy::unwrap_used)]

use crate::UserModel;

#[test]
fn tracks_only_the_changed_cells() {
    let mut model = UserModel::new_empty("model", "en", "UTC", "en").unwrap();
    model.set_track_changes(true);

    model.set_user_input(0, 1, 1, "5").unwrap(); // A1 = 5
    model.set_user_input(0, 1, 2, "=A1*2").unwrap(); // B1 = 10
    model.set_user_input(0, 1, 3, "99").unwrap(); // C1 = 99
    model.take_changed_cells(); // drain

    // Editing A1 changes A1 and its dependent B1, but not C1. The result is
    // ordered by position.
    model.set_user_input(0, 1, 1, "6").unwrap();
    let changed: Vec<_> = model
        .take_changed_cells()
        .into_iter()
        .map(|c| (c.row, c.column, c.value))
        .collect();
    assert_eq!(
        changed,
        vec![(1, 1, "6".to_string()), (1, 2, "12".to_string())]
    );

    // Setting the same value produces no change.
    model.set_user_input(0, 1, 1, "6").unwrap();
    assert!(model.take_changed_cells().is_empty());
}

#[test]
fn no_tracking_by_default() {
    let mut model = UserModel::new_empty("model", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 1, "5").unwrap();
    assert!(model.take_changed_cells().is_empty());
}
