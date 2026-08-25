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

/// A function implementation must read cell state through the tracing
/// accessors, so the read is recorded as a dependency edge. The raw path is
/// `self.workbook`, a `pub` field.
///
/// This one stays a grep-gate. `Model`'s own raw getters (`fetch_cell`,
/// `get_cell_value`) are already out of reach: they are private to
/// `crate::model`, and `functions/` is not a descendant of it, so calling one
/// does not compile. What is left reachable is the `pub` `workbook` field and
/// the `pub` untraced API on `Model` — and neither is expressible as a
/// `disallowed-methods` entry at a sane cost: a field is not a method, and the
/// getters underneath it have 35 to 687 call sites across the workspace
/// (`Worksheet::cell` alone has 57), so every candidate ban would need dozens
/// of allow sites and would stop meaning anything.
///
/// The structural fix is a receiver change, not a lint: give `functions/` a
/// `TracedModel` newtype wrapping `&mut Model` that exposes the tracing
/// accessors and no `workbook`, and move the function `impl`s onto it. Then
/// this gate can be deleted outright.
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
/// structural rebuild (a cell, row or column move) legitimately rewrites the
/// anchor and then the placeholders of the rectangle the anchor has just
/// re-declared, so it must suspend the guard — through the scoped
/// `with_cse_guard_suspended`, which restores the flag on every exit path.
///
/// That a rebuild path cannot suspend the guard any *other* way is no longer
/// checked here: the flag is a private field of `model::cse_guard`, so a
/// hand-rolled set-then-reset pair — the one that leaks the suspension on an
/// early `?` between its halves — does not compile. What privacy cannot say
/// is that a future `*_unchecked` rebuild path must reach for the scope at
/// all. That positive obligation is what is left of this gate.
#[test]
fn unchecked_rebuild_paths_suspend_the_cse_member_guard() {
    let actions = include_str!("../actions.rs");
    // Functions allowed to write cells without entering the scope, each with
    // the reason it is safe. Add here only with a comment saying why the path
    // can never write into a CSE rectangle mid-rebuild.
    const ALLOWLIST: &[&str] = &[];
    let items = fn_items(actions);
    let mut rebuild_writers = 0;
    for (name, body) in &items {
        let writes_cells = body.contains("set_user_input(")
            || body.contains("set_user_array_formula(")
            || (name != "rebuild_moved_cells" && body.contains("rebuild_moved_cells("))
            || (name != "move_cell_write" && body.contains("move_cell_write("));
        if !writes_cells || ALLOWLIST.contains(&name.as_str()) {
            continue;
        }
        let is_rebuild_path = name.ends_with("_unchecked")
            || (name != "rebuild_moved_cells" && body.contains("rebuild_moved_cells("))
            || (name != "move_cell_write" && body.contains("move_cell_write("));
        if !is_rebuild_path {
            continue;
        }
        rebuild_writers += 1;
        assert!(
            body.contains("with_cse_guard_suspended("),
            "{name} in actions.rs rewrites cells during a structural rebuild \
             but never suspends the CSE member guard; a CSE anchor in the \
             moved cell/row/column will make the rebuild error. Wrap the \
             write-back in with_cse_guard_suspended (see move_cell) or \
             allowlist the fn here with a reason."
        );
    }
    assert!(
        rebuild_writers >= 3,
        "expected at least move_cell, move_column_unchecked and \
         move_row_unchecked to be checked; the gate no longer sees the \
         rebuild paths and must be updated"
    );
}

/// `range_clear_all` tears down the whole spill of a dynamic array reached
/// from the cleared range, but only the cells inside the range were selected:
/// the ones outside must keep their style. The one footprint-teardown
/// primitive is `Worksheet::clear_array_footprint`, built on the
/// style-preserving `cell_clear_contents` (it materializes `EmptyCell { s }`;
/// `remove_cell` drops the style).
///
/// The "do not reach for `remove_cell`" half of that rule is no longer a
/// grep-gate: `clippy.toml` bans `Worksheet::remove_cell` workspace-wide, so a
/// new call anywhere fails the build until someone writes an explicit
/// `#[allow(clippy::disallowed_methods)]` with a justification. What clippy
/// cannot say is "this function must call that helper", or "only
/// `clear_array_footprint` may reach for `cell_clear_contents`" — a positive
/// obligation and a one-caller restriction. Those two clauses stay here.
#[test]
fn range_clear_all_spill_teardown_preserves_style() {
    let model = include_str!("../model/mod.rs");
    let body = fn_items(model)
        .into_iter()
        .find(|(name, _)| name == "range_clear_all")
        .map(|(_, body)| body)
        .expect("range_clear_all must exist in model/mod.rs (update this gate if renamed)");
    assert!(
        body.contains("clear_array_footprint("),
        "range_clear_all must tear the spill footprint down through \
         Worksheet::clear_array_footprint so cells outside the cleared range \
         keep their style"
    );
    assert!(
        !body.contains("cell_clear_contents("),
        "range_clear_all must not hand-roll the footprint teardown; go \
         through Worksheet::clear_array_footprint"
    );

    // The helper itself must stay on the style-preserving primitive, or the
    // guarantee above is hollow. (That it does *not* use `remove_cell` is
    // clippy's job now.)
    let worksheet = include_str!("../worksheet.rs");
    let helper = fn_items(worksheet)
        .into_iter()
        .find(|(name, _)| name == "clear_array_footprint")
        .map(|(_, body)| body)
        .expect("clear_array_footprint must exist in worksheet.rs (update this gate if renamed)");
    assert!(
        helper.contains("cell_clear_contents("),
        "clear_array_footprint must clear with the style-preserving \
         cell_clear_contents"
    );
}
