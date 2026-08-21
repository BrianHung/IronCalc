//! Construction-time checks that incremental recalc invariants hold in source.

#[test]
fn graph_is_only_notified_by_the_journal() {
    let model = include_str!("../model/mod.rs");
    let actions = include_str!("../actions.rs");
    let undo = include_str!("../user_model/undo_redo.rs");
    let clipboard = include_str!("../user_model/clipboard.rs");
    let common = include_str!("../user_model/common.rs");
    assert!(
        !model.contains("fn mark_value_edit"),
        "mark_value_edit must not exist; the journal consumer dirties the graph"
    );
    assert!(
        !model.contains("fn force_full_recompute"),
        "force_full_recompute must be invalidate_graph"
    );
    for (name, src) in [
        ("actions.rs", actions),
        ("undo_redo.rs", undo),
        ("clipboard.rs", clipboard),
        ("common.rs", common),
    ] {
        assert!(
            !src.contains("graph.mark_dirty("),
            "{name} must not mark the graph dirty; writes go through the journal"
        );
        assert!(
            !src.contains("force_full_recompute"),
            "{name} must not call force_full_recompute"
        );
    }
    let drain_marks = model.matches("self.graph.mark_dirty(").count();
    assert_eq!(
        drain_marks, 3,
        "model/mod.rs should mark_dirty only from drain_write_journal (cell + hidden + FormulaText)"
    );
}
