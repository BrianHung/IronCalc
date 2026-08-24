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

/// Extracts every `fn` item in `src` as (name, body-with-signature), by brace
/// counting from each `fn` line. Good enough for grep-gates: a parse drift
/// fails the calling assertion loudly rather than passing silently.
fn fn_items(src: &str) -> Vec<(String, String)> {
    let mut items = Vec::new();
    let bytes = src.as_bytes();
    let mut search = 0;
    while let Some(found) = src[search..].find("fn ") {
        let at = search + found;
        // Only real item starts: `fn`, `pub fn`, `pub(crate) fn` at the start
        // of a line, not the word inside a comment or a string.
        let line_start = src[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let prefix = src[line_start..at].trim();
        if !(prefix.is_empty() || prefix == "pub" || prefix.starts_with("pub(")) {
            search = at + 3;
            continue;
        }
        let name_end = src[at + 3..]
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map(|i| at + 3 + i)
            .unwrap_or(src.len());
        let name = src[at + 3..name_end].to_string();
        // The body opens at the first `{` past the signature.
        let Some(open_rel) = src[name_end..].find('{') else {
            search = at + 3;
            continue;
        };
        let open = name_end + open_rel;
        let mut depth = 0usize;
        let mut end = open;
        for (i, &b) in bytes[open..].iter().enumerate() {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        items.push((name, src[at..end].to_string()));
        search = end.max(at + 3);
    }
    items
}

/// The CSE member guard blocks user writes into an array rectangle. A
/// structural rebuild (a row or column move) legitimately rewrites the anchor
/// and then the placeholders of the rectangle the anchor has just re-declared,
/// so it must suspend the guard, the way `move_cell` and `rebuild_moved_cells`'s
/// callers do. Nothing in the type system forces a future `*_unchecked`
/// rebuild path to do the same: this gate does.
#[test]
fn unchecked_rebuild_paths_suspend_the_cse_member_guard() {
    let actions = include_str!("../actions.rs");
    // Functions allowed to write cells without referencing the guard, each
    // with the reason it is safe. Add here only with a comment saying why the
    // path can never write into a CSE rectangle mid-rebuild.
    const ALLOWLIST: &[&str] = &[];
    let items = fn_items(actions);
    let mut rebuild_writers = 0;
    for (name, body) in &items {
        let writes_cells = body.contains("set_user_input(")
            || body.contains("set_user_array_formula(")
            || (name != "rebuild_moved_cells" && body.contains("rebuild_moved_cells("));
        if !writes_cells || ALLOWLIST.contains(&name.as_str()) {
            continue;
        }
        let is_rebuild_path = name.ends_with("_unchecked")
            || (name != "rebuild_moved_cells" && body.contains("rebuild_moved_cells("));
        if !is_rebuild_path {
            continue;
        }
        rebuild_writers += 1;
        assert!(
            body.contains("cse_member_guard_suspended"),
            "{name} in actions.rs rewrites cells during a structural rebuild \
             but never suspends cse_member_guard_suspended; a CSE anchor in the \
             moved row/column will make the rebuild error. Suspend the guard \
             around the write-back (see rebuild_moved_cells) or allowlist the \
             fn here with a reason."
        );
    }
    assert!(
        rebuild_writers >= 2,
        "expected at least move_column_unchecked and move_row_unchecked to be \
         checked; the gate no longer sees the rebuild paths and must be updated"
    );
}

/// `range_clear_all` tears down the whole spill of a dynamic array reached
/// from the cleared range, but only the cells inside the range were selected:
/// the ones outside must keep their style. `cell_clear_contents` preserves it
/// (it materializes `EmptyCell { s }`); `remove_cell` drops it. The only
/// `remove_cell` in the function must stay the in-range sweep.
#[test]
fn range_clear_all_spill_teardown_preserves_style() {
    let model = include_str!("../model/mod.rs");
    let body = fn_items(model)
        .into_iter()
        .find(|(name, _)| name == "range_clear_all")
        .map(|(_, body)| body)
        .expect("range_clear_all must exist in model/mod.rs (update this gate if renamed)");
    assert_eq!(
        body.matches("remove_cell(").count(),
        1,
        "range_clear_all must call remove_cell exactly once (the in-range \
         sweep); the out-of-range spill footprint is torn down with the \
         style-preserving cell_clear_contents"
    );
    assert!(
        body.contains("cell_clear_contents("),
        "range_clear_all must tear the spill footprint down with \
         cell_clear_contents so cells outside the cleared range keep their style"
    );
}
