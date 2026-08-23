//! Construction-time checks that incremental recalc invariants hold in source.
#![allow(clippy::unwrap_used, clippy::expect_used)]

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
        drain_marks, 4,
        "model/mod.rs should mark_dirty only from drain_write_journal (cell + link + hidden + FormulaText)"
    );
}

#[test]
fn functions_do_not_touch_the_workbook_directly() {
    let functions = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/functions");
    let mut hits = Vec::new();
    fn walk(dir: &std::path::Path, hits: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, hits);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap();
            for (i, line) in src.lines().enumerate() {
                if line.contains("self.workbook.")
                    || line.contains("self.workbook[")
                    || line.contains("&self.workbook")
                {
                    hits.push(format!("{}:{}:{line}", path.display(), i + 1));
                }
            }
        }
    }
    walk(&functions, &mut hits);
    assert!(
        hits.is_empty(),
        "functions/ must read through recording accessors, found:\n{}",
        hits.join("\n")
    );
}
