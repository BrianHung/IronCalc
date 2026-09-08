# Incremental recalc — reviewer's guide to the PR stack

For a reviewer coming in cold. Read in order; budget ~1 day for a full pass. Everything here is verifiable
with the commands given — do not take the narrative on faith, including mine.

## 1. What you are reviewing

IronCalc recomputes the whole workbook on every `evaluate()`. This stack adds an **opt-in** incremental
mode that recomputes only the cells an edit can affect, plus a changed-cells delta for UIs. The default
stays full recalculation; production behaviour must be bit-identical to `main` unless the mode is opted
into.

| PR | Branch | Base | Head (as of this guide) | What it adds |
|---|---|---|---|---|
| [#90](https://github.com/BrianHung/IronCalc/pull/90) | `incremental-recalc-fanout-guard` | `main` | `f75148ad` | The engine: observed-dependency graph, write journal, scheduler, `take_changed_cells`, Verify oracle, differential fuzzer. (#89 "changed cells" was folded into this PR.) |
| [#93](https://github.com/BrianHung/IronCalc/pull/93) | `incremental-recalc-range-composition` | #90 | `64bc24a0` | `SUM/MIN/MAX/COUNT` over overlapping ranges reuse scan-order prefix folds (bit-identical to Full), Incremental/Verify only. |
| [#94](https://github.com/BrianHung/IronCalc/pull/94) | `incremental-recalc-lazy-crud` | #90 | `fc6ce848` | Row/column insert/delete rewrite formula ASTs in place instead of stringify-and-reparse. Sibling of #93. |

**Merge order:** #90, then #93 and #94 independently, **merge commits only** (squash/rebase of #90 rewrites
the SHAs the children contain).

(Resolved 2026-08-24: #93 is retargeted to `incremental-recalc-fanout-guard` and all three PR descriptions
describe the three-PR stack.)

## 2. The two questions this review answers

1. **Incremental ≡ Full.** After any sequence of edits, does an Incremental model show exactly the values,
   types, links, conditional formats, and spill extents a Full model shows? And is the `take_changed_cells`
   delta complete (every moved cell listed) and sound (nothing unchanged listed)?
2. **No default-mode change.** With `RecalcMode::Full` (the default), is every output bit-identical to
   `upstream/main`? This stack touched `SUM/MIN/MAX/COUNT`, displacement, `FormulaValue`, xlsx export, and
   `evaluate_cell` — all on the Full path.

Code quality is a third, lower-priority lens; §8 gives the bar.

## 3. Architecture in one page (what to hold in your head)

Four mechanisms, each replacing a convention that caused bugs in earlier rounds. Verify each exists and is
the *only* mechanism of its kind.

**A. Dependencies are observed, not predicted** — `base/src/recalc/trace.rs`, `model/mod.rs::evaluate_cell`.
When a formula starts evaluating, a `ReadSet` frame is pushed (`read_stack.push` at `evaluate_cell`, line
~1707). Every read during evaluation records into the top frame: `trace_cell` at the entry of
`evaluate_cell` (every cell read passes through here — there is no other read path from `functions/`),
`trace_rect` for ranges (one edge for `SUM(A:A)`, not a million), `trace_input` for non-cell inputs (`Input::
{RowHidden, OwnCoord, FormulaText, Name, Clock, Random, Environment, Computed, SheetStructure}`). When the
formula finishes, `commit_reads` → `DependencyGraph::replace_reads` swaps its edges atomically; the
`precedents` reverse index makes removal O(degree). There is **no static AST walk**; `IF` records only the
taken branch; `OFFSET`/`INDIRECT` record their actual target plus `Input::Computed` so a structural edit
re-runs them instead of shifting a stale snapshot; `SUMIFS` records exactly the criteria cells
`run_ifs` reads. Tracing is on only when `recalc_mode != Full` (`tracing()`), so Full pays nothing.

**B. Writes are journaled** — `base/src/recalc/journal.rs`, `worksheet.rs`, `model/mod.rs::drain_write_journal`.
Every `Worksheet` mutator pushes `Write::Cell { at, was_formula, is_formula }` or `Write::Hidden` into the
sheet's `write_log`; `Model::evaluate` and `take_changed_cells` drain all sheets and derive dirtiness
(`mark_dirty`, `remove_dependent` when a formula was overwritten, re-dirty `FormulaText` readers). Nothing
else may call `graph.mark_dirty`; `test_recalc_invariants.rs::graph_is_only_notified_by_the_journal` greps
for it. Structural edits (insert/delete rows/cols) are not cell writes and go through
`actions.rs::record_structural_edit` → `graph.structural_edit`, which shifts every stored position and
dirties readers of moved precedents, `OwnCoord`/`FormulaText`/`Name`/`Computed` inputs on that sheet, and
blocked `#SPILL!` anchors. Three non-cell invalidations remain and are named: `invalidate_graph()` from
`reset_parsed_structures` (sheet add/delete/rename, defined-name CRUD), `set_locale`, `set_timezone`;
row/column *moves* force full in `record_structural_edit`.

**C. Stored values equal observed values** — blank formula results are coerced to `Number(0)` **at the
formula-result boundary** (`evaluate_cell` returns to same-pass readers exactly what `set_cells_with_result`
stores). A reader's result no longer depends on whether it ran before or after its precedent, Full is
order-independent, Incremental can serve stored values to out-of-cone readers, and the semantics match
`main`/Excel (`=A1` over blank displays `0`, `ISBLANK` FALSE, xlsx caches `<v>0</v>`). An earlier attempt
(`FormulaValue::Empty`, propagating emptiness) changed Excel semantics and was reverted — if you see `Empty`
references in docs/comments they are stale.

**D. The scheduler is the only mode-dependent code** — `model/incremental.rs::evaluate_selective` (~130 lines).
Seeds = drained journal writes ∪ `always_dirty_cells()` (readers of `Random`/`Clock`/`Environment`). Cone =
`reachable(seeds)` over cell, rect, and input edges. Fallbacks to a Full pass: graph not ready (first pass,
`invalidate_graph`), fanout above half the formulas (perf only; Verify disables it), any array/spill anchor in
the cone, or a new dynamic array created during the pass. Otherwise `topo_order` + `recompute_frontier` with
early cutoff (a cell whose `ChangeKey` — type, value bits, link — did not move stops propagation). Formula
edits stay incremental: the new formula's reads are recorded the first time it runs.

**Oracles.** `RecalcMode::Verify` (`--features recalc_verify`) runs Incremental, asserts delta completeness
and soundness, asserts every stored formula value equals a live re-evaluation
(`assert_stored_matches_live`), then runs a Full pass on a **snapshot** and restores the Incremental state so
the oracle cannot heal the graph it checks. `base/tests/fuzz_differential.rs` + `common/mod.rs` run random
op sequences on a Full and an Incremental model in lockstep and compare every cell after every evaluate, with
a ddmin minimizer.

## 4. Read order (files, with what to verify in each)

1. `base/src/recalc/{mod,trace,journal}.rs` (130 lines). Check: `ReadSet::record_cell` is suppressed by a
   covering rect; `WriteLog::recording` — find every `set_recording(false)` caller and confirm each one is a
   pure evaluation write (storing results), never a user edit.
2. `base/src/model/mod.rs::evaluate_cell` (~line 1590–1780). Check: the frame push happens after the
   `CellState::Evaluating` insert and the pop+commit happens on **both** exits (success and the
   `set_cells_with_result` error path); `trace_cell` runs before the out-of-scope early return; the
   `recompute_scope` early return serves stored values — with `FormulaValue::Empty` this is lossless.
3. `base/src/model/mod.rs`: `trace_rect` call sites (`RangeKind`, `DefinedNameKind` range, `evaluate_range`,
   `fn_offset`/`fn_indirect`), `sheet_dimension`/`row_hidden` recording accessors, `drain_write_journal`,
   `set_cells_with_result` (blocker read is traced: `self.trace_cell(sheet, r, c)` before returning
   `#SPILL!`).
4. `base/src/functions/mod.rs::evaluate_function` head: the `Input` classification by `Function`. This is
   a small `match` with a `_ => {}` default — **it is the one remaining whitelist**. Enumerate every
   `Function` that reads something other than cell values (`CELL`, `INFO`, `SHEET`, `SHEETS`, `ROW`,
   `COLUMN`, `ROWS`, `COLUMNS`, `FORMULATEXT`, `ISFORMULA`, `NOW`, `TODAY`, `RAND*`, `SUBTOTAL`, `HYPERLINK`?)
   and confirm each records an input. `functions_do_not_touch_the_workbook_directly` enforces that reads go
   through accessors; it does not enforce that every accessor records.
5. `base/src/dependency_graph.rs` (780 lines): `SheetRanges` band index (unit-tested against brute force),
   `replace_reads`/`remove_dependent`/`precedents`, `topo_order` (successors sorted → deterministic),
   `reachable`, `structural_edit`/`shift` (confirm `precedents` ReadSets are shifted too, line ~688),
   `GraphState`.
6. `base/src/model/incremental.rs` (580 lines): `evaluate_selective`, `recompute_frontier`,
   `verify_incremental_matches_full`, `assert_stored_matches_live`, `take_changed_cells`, CF diffing.
7. `base/src/worksheet.rs` journal producers (7 sites) and `update_cell`; `sheet_data` visibility.
8. `base/src/actions.rs`: `record_structural_edit`, `move_cell`, `displace_cells`,
   `reset_dynamic_array_spills`.
9. `base/src/test/test_recalc_invariants.rs`, `base/src/test/user_model/test_incremental_recalc.rs`
   (~1,900 lines; every bug from §7 has a named test here — spot-check five against the table).
10. #93: `base/src/model/range_reduce.rs`, the four call sites in `functions/math_and_trigonometry/
    mathematical.rs` and `statistical/count_and_average.rs`, generation stamps (`965842cd`).
11. #94: `base/src/expressions/parser/displace.rs`, `stringify.rs::displace_resolved_coordinates`,
    `actions.rs::set_displaced_formula` (Scalar gate → string-path fallback; quote-prefix strip; 15-digit
    rounding).
12. Bindings: `bindings/wasm/src/lib.rs` (`recalcMode` ctor arg, `takeChangedCells` tagged union),
    `types.ts`, `tests/test.mjs`; Python/Node are Full-only and say so.

## 5. Run this before reading code

```bash
source "$HOME/.cargo/env"
git fetch origin incremental-recalc-fanout-guard incremental-recalc-range-composition incremental-recalc-lazy-crud
git merge-base --is-ancestor origin/incremental-recalc-fanout-guard origin/incremental-recalc-range-composition && echo "#90 in #93"
git merge-base --is-ancestor origin/incremental-recalc-fanout-guard origin/incremental-recalc-lazy-crud && echo "#90 in #94"
git worktree add /tmp/rv-90 origin/incremental-recalc-fanout-guard   # repeat for -93, -94, and upstream/main

cd /tmp/rv-93   # contains #90 + #93; run the same in rv-94
cargo test -p ironcalc_base --lib                                               # default Full
IRONCALC_RECALC=incremental cargo test -p ironcalc_base --lib                   # every test forced Incremental
IRONCALC_RECALC=verify cargo test -p ironcalc_base --features recalc_verify --lib   # oracle
FUZZ_SEEDS=100 FUZZ_STEPS=200 cargo test -p ironcalc_base --test fuzz_differential -- --nocapture
FUZZ_SEEDS=40 FUZZ_STEPS=150 cargo test -p ironcalc_base --features recalc_verify --test fuzz_differential -- --nocapture
cargo clippy -p ironcalc_base --all-targets --all-features -- -D warnings
cargo test -p ironcalc                                                          # xlsx crate
cd bindings/wasm && make tests                                                  # wasm-pack + node tests
```

Then, on an x86_64 machine or under Rosetta (`--target x86_64-apple-darwin`), rerun the Full suite: the
signed-zero `MIN/MAX` case is invisible on arm64.

Note the CI fuzz job (`build-test.yaml:52`) sets `FUZZ_AVOID_OPS=InsertRows,DeleteRows,InsertCols,DeleteCols,
MoveRows,MoveCols,AddSheet,DeleteSheet,RenameSheet` — it **excludes every structural op**, which is where most
historical bugs were. The local run above does not; expect it to be the most informative thing you do.
Treat any divergence the harness does *not* tag `FLAKY` as a bug; `FLAKY`-tagged CSE-array + column-insert
shapes are a pre-existing upstream nondeterminism (verify whether the sorted-iteration commits fixed it).

## 6. Hunt list — what to try to break, by mechanism

### 6.1 Reads that could escape the tracer (→ stale Incremental values)
- A function reading workbook state through a path that does not record: anything in `functions/` reaching
  `self.workbook_name()`, tables, `parsed_defined_names`, styles (quote prefix affects text), row/column
  *styles* (hidden lives on the row style — `delete_row_style`/`set_row_style` must journal `Hidden` if the
  flag changes), sheet dimension (value-neutral clamp — argue why), `links`, `cf_cache`.
- Sheet **index** renumbering: `Position` is `(sheet_index, row, col)`; `delete_sheet`/`insert_sheet` shift
  indices of later sheets. Confirm `reset_parsed_structures` → `invalidate_graph()` runs on every sheet
  insert/delete/move and that nothing caches a `Position` across it (`links`, `cf_cache`, `changed_cells`
  delta!). Try: two sheets, formula on sheet 2 reads sheet 1, insert a sheet at index 0, edit, read the delta.
- Conditional formatting rules have no edges; confirm CF is re-evaluated unscoped every Incremental pass and
  that `record_cf_changes` diffs the whole cache.
- Implicit intersection (`@A1:A5`) reads the formula's own coordinates — is `OwnCoord` recorded? Insert a
  row above it.
- Names: global vs sheet-scoped with the same text; a name redefined from a cell to a range between passes;
  a name deleted while referenced; a name used inside a `LAMBDA` body; a name whose target is on another
  sheet than the structural edit.
- `SUBTOTAL(1xx)` over rows hidden via `UserModel::set_rows_hidden` and via undo; `SUBTOTAL` with a computed
  function number; `AGGREGATE` if implemented.
- A spill whose blocker is *itself* a formula that becomes empty; a spill freed by `delete_rows` of the
  blocker; `=E15#` spill-range operator (needs two passes on `main`).
- Three or more Incremental passes in a row in pure `RecalcMode::Incremental` (not Verify): `IF` branch flips
  so the read set shrinks then grows; `OFFSET` target moves twice; `INDIRECT` built from a cell that changes
  sheet names.

### 6.2 Writes that could escape the journal (→ nothing dirtied)
- Every `worksheet_mut(` caller in `actions.rs`, `user_model/{undo_redo,clipboard,common,autofill}.rs`,
  `new_empty.rs`, `model/mod.rs`: does each mutate through a journaling method? `sheet_data` is private —
  confirm no `pub(crate)` accessor returns `&mut` to it.
- `apply_external_diffs`, paste (cut vs copy), autofill over spill anchors, `paste_csv_string` partial
  failure, undo of a structural edit, redo.
- `set_recording(false)` windows: an edit performed while recording is off is invisible to the graph.
- xlsx import into an existing model; `from_bytes` (does a deserialized model start `MustRebuild`?).
- Style-only writes must **not** dirty (perf), except quote-prefix and hidden.

### 6.3 Default-mode bit-identity with `main` (disqualifying if violated)
- `SUM/MIN/MAX/COUNT` in `RecalcMode::Full` go through `fold_range` → `reduce_range_direct`: one accumulator,
  row-major, `b.min(a)` operand order. Test `SUM(5, A1:B2, C1)`, errors mid-range (precedence, short-circuit),
  booleans/text/empties, full-column ranges (clamping differs between `fn_sum` and `fn_min`), ±0, NaN, an
  all-empty `MIN`.
- Displacement (#94): fast path must equal `parse(render_displaced(ast))` for every formula × displacement ×
  context; the module's own test asserts `Node` equality. Imported Excel plain formulas that static-analyze
  non-`Scalar` (names, `LET`, `A1:INDEX`, `A1#`) must take the string path. A reference pushed past
  `LAST_ROW` becomes `#REF!` in both paths (intentional change vs `main`, which produced `#NAME?` — flag if
  undocumented).
- `FormulaValue::Empty` round-trips through xlsx export/import and `to_bytes`/`from_bytes`; `xlsx/src/
  compare.rs` treats `Empty` and an absent value the same way `main` did.
- `evaluate_full` in Full mode ends with `force_full()` (graph never `Ready`), so no bookkeeping leaks.

### 6.4 Scheduler and delta
- Delta soundness across the Verify internal passes; after a redundant `evaluate()` with no edits
  (`Cells([])`, not `Everything`, unless `RAND`/`NOW` present); after a trailing-row delete with nothing below
  (must be `Everything`); a `HYPERLINK` whose URL changes under a fixed label (must be listed — `ChangeKey`
  includes the link); +0/−0 flips.
- Fanout guard: `formula_cell_count` maintenance after formula→value overwrites and deletes.
- Cycle in the cone → `recompute_all` → `#CIRC!` matches Full.

### 6.5 Performance (the feature's reason to exist)
`base/tests/review_bench.rs` (in the review worktrees; copy it): a 600-cell cone must cost the same with 2k,
8k, and 32k unrelated formulas, and less than Full at every size. Any super-linear growth means a per-commit
scan crept back into `commit_reads`/`remove_dependent`/`topo_order`.

## 7. History: every bug found so far, and the test that pins it

Twenty-one confirmed bugs across four review rounds, all fixed at the heads above. Each has a named test in
`test_incremental_recalc.rs`; the consolidated `base/tests/review_probes.rs` in the review worktrees re-runs
them as Full-vs-Incremental comparisons. Use this table to check that a fix is a *mechanism*, not a patch.

| Round | Bug | Root-cause class | Now prevented by |
|---|---|---|---|
| 1 | SUMIFS reads criteria outside the declared range | static read model incomplete | observed reads (A) |
| 1 | Name edge shifted on insert while definition is not displaced | static model | `Input::Name` re-dirty on structural edit (A+B) |
| 1 | SUBTOTAL stale after hide + batched edit | unmodelled input | `Input::RowHidden` per scanned row + `Write::Hidden` |
| 2 | SUM re-association changed default-Full values | two implementations | `reduce_range_direct` in Full |
| 2 | COUNT cached a mid-cycle `#CIRC` | cache validity | `circ_seen` + pass generation |
| 2 | Overwritten `RAND` reported forever | mutation without notification | journal `was_formula` → `remove_dependent` |
| 2 | Spill-anchor overwrite left spill dependents stale (regression from a fix) | mutation | arrays role retained → Full fallback |
| 3 | Rejected write to a missing sheet → panic next pass | validation after notification | journal only records successful writes |
| 3 | Blocked `#SPILL!` anchor not re-run when insert frees it | static model | blocker read is traced; `reset_dynamic_array_spills` dirties blocked anchors |
| 3 | Composed aggregate cached before a spill landed | cache validity | cache cleared on array write + generation |
| 3 | Dyn-cone cell over-reported, nondeterministically | hash-order + two phases | no second phase; sorted successors |
| 3 | x86 signed-zero `MIN/MAX` differs from main | operand order | `b.min(a)`; arm64+x86 CI |
| 3 | #94 quote-prefix kept / constants unrounded | two implementations | strip + `to_excel_precision(15)` |
| 3 | Empty pass-through stored as `0` | lossy storage | `FormulaValue::Empty` (C) |
| 3 | `*IFS` with `INDEX`/`LET` ranges untracked | static model | observed reads (A) |
| 4 | Later-precedent scan: O(cone×F) and wrong on #93 | patch-on-patch | deleted with (C) |
| 4 | SUMIFS over undefined names always reported | "volatile" conflated with "report" | `always_dirty` derived from `Random/Clock/Environment` only |
| 4 | `remove_dependent` O(E) per evaluated cell | missing reverse index | `precedents` |

## 8. Code-quality bar

Score 1–10 on: (a) every invariant above is enforced by a type, a test, or a compiler error — not a comment;
(b) one implementation per semantic (no fast/slow path without an in-module equivalence test); (c) the
only `recalc_mode` branches outside `evaluate_selective` are `tracing()` and the Full-mode fold in
`range_reduce`; (d) tuple coordinates (`Position`, `Area`) and the `#[cfg(feature)]`-gated `Verify` variant
on the public `RecalcMode` (no `#[non_exhaustive]`) are known debts — flag, don't block; (e) `WriteLog` is
`pub use`d from `lib.rs` — ask whether that is intended public API.

## 8b. Measured status at these heads (2026-08-22) — start from here, then try to disprove it

All three tips: default Full / forced Incremental / Verify suites green (2366 / 2383 / 2376 tests), xlsx crate
green, wasm crate builds, `clippy --all-targets --all-features -D warnings` clean, the 18-probe regression
file (`review_probes.rs`, every bug in §7) green, the imported-workbook displacement repro green, and the
CI-shaped fuzz (structural ops excluded) green at 40×120 on every tip. Perf: a 600-cell cone costs 7 ms with
32k unrelated formulas (Full: 41 ms); the wide-cone bench is at or below Full at every size.

### Disqualifying at these heads (verified by hand, not just by an agent)

**D1 — `FormulaValue::Empty` changes default-Full semantics vs `main` and vs Excel (#90).** A formula
whose result is an empty cell now *propagates* emptiness instead of coercing to `0`:

| `A2 = =A1`, A1 blank | main / Excel | #90 `e448d15d` |
|---|---|---|
| A2 displays | `0` | `""` |
| `ISBLANK(A2)` | FALSE | TRUE |
| `COUNT(A2)` | 1 | 0 |
| `COUNTBLANK(A2)` | 0 | 1 |
| `A2&"x"` | `"0x"` | `"x"` |

xlsx export writes `<c r="A2"><f>A1</f></c>` with no `<v>` (main writes `<v>0</v>`); reloading shows
`#ERROR!` until the first evaluate, and any data-only reader (openpyxl, pandas) sees `None` where main gave
`0`. The order-independence goal behind `Empty` is right; the mechanism is wrong. The fix is to coerce at the
formula-result boundary: `evaluate_cell` returns the same `Number(0)` to same-pass readers that
`set_cells_with_result` stores, so Full is order-independent *and* Excel-compatible, `Empty` is unnecessary,
and the xlsx output returns to `main`'s. Probe: `scratchpad/wt-90/base/tests/probe_empty.rs` (+ `xlsx/tests/
probe_empty_xlsx.rs`), same file on `wt-main` for the baseline.

**D2 — Per-pass cost grows without bound over a model's lifetime (#90, `dependency_graph.rs:166-191`).**
`SheetRanges::insert` indexes an area into `bands`/`wide` on "first sight", and `remove_dependent` drops the
`dependents` key when its set empties but leaves the index entry; every `replace_reads` therefore re-indexes the
same area, and `containing()` scans the duplicates forever. `B1==SUM(A1:A3)`, edit A1 + `evaluate()` repeatedly:
5k-pass windows take **2.3 s, 6.8 s, 11.2 s, 15.5 s** (release). A long-lived UI model degrades
continuously. Fix: prune the band/wide entry when the area's dependent set empties (or make first-sight check
the index). Probe: `review_audit_b.rs::b1_*`.

**D3 — CSE array range cells are not covered by the arrays→Full guard (#90, `incremental.rs::collect_array_cells`).**
`graph.arrays` is derived from cell *kinds*, not from each anchor's declared `r`; after `delete_columns` drops a
CSE member (itself a pre-existing upstream bug), a new formula reading the member stays incremental:
`set_user_array_formula(0,4,8,1,2,"=A1:A3+1")`; `delete_columns(0,7,1)`; `G5==IFERROR(F18,-1)` → Full `1`,
Incremental blank. Fix: insert every position in each anchor's `r` rectangle into `arrays`. Probe:
`review_audit_b.rs::b3_*`.

### Should fix before merge

- **Link edits are not journaled** (`links.rs::set_cell_link`/`delete_cell_link`, and worksheet-link
  displacement in `actions.rs`): the delta misses them although the link is part of `ChangeKey`. Minimal:
  Incremental model, `set_cell_link(0,1,1,…)`, `evaluate()` → `Cells([])`. Add `Write::Link` + consumer.
  Probe: `review_links.rs`.
- **Anything not provably `Scalar` at parse time forces a Full pass every time it is in the cone**
  (`mod.rs` `is_dynamic = !Scalar` → `ArrayKind::Dynamic` → `is_dynamic_array_anchor` → Full): `=LET(…)`,
  `=LAMBDA(x,x+A1)(3)`, `=INDEX(A1:A3,1)`, named lambdas. Measured with `take_changed_cells()==Everything`.
  Not a correctness bug, but it silently removes the feature for common formulas. Treat a Dynamic anchor whose
  last result was 1×1 as scalar for the guard. Probe: `review_audit_b.rs::b5_*`.
- **#93 re-association is not "one rounding step"**: `SUM(A1:B4)` of `0.1,0.2,0.3,0.7,1e16,1,-1e16,0.1` is
  `4.1` in Full/main and `2.0` in Incremental (cancellation). The harness's 1e-9 *relative* tolerance hides
  this. Either compose along the same per-cell row-major order as the direct scan (then both modes are
  bit-identical and the documented contract disappears) or state the contract honestly as "may differ
  arbitrarily under cancellation". The `RecalcMode::Incremental` doc for this lives in #90 but describes #93.
- Reported by the tracer audit, code-plausible, not independently reproduced: per-evaluation state
  (`self.support`, `self.lambdas`, `variable_stack`) is cleared only in `evaluate_full`, so Incremental passes
  leak (heap 336→912 KB over 20k passes of `=A1+1`); `evaluate_selective` re-walks all `sheet_data` in
  `collect_array_cells` and scans all `input_dependents` keys each pass (cost ∝ total cells: 1.6 → 5.9 ms for
  a fixed cone at 2k → 32k cells — below Full, but not flat); Verify's `assert_stored_matches_live` false-
  positives on range-based cycles (a simple `A1↔B1` cycle passes).

The **structural-inclusive** fuzz (150 seeds × 250 steps, the configuration CI does *not* run) produced
ten deterministic signatures, identical on all three tips (#93 and #94 add none). Every value divergence
reproduces on `upstream/main` in Full mode and **Incremental is the correct side** — they are pre-existing CSE
array bugs that the sorted-iteration commits turned from flaky into deterministic. Triage so you do not
re-derive it:

| Fuzz seed | What | Triage | Status |
|---|---|---|---|
| 60, 18 | A cell whose **link** changes (cleared, or created by a style write) is missing from the delta after structural edits | Link edits are not journaled (see "Should fix" above); additionally static links are duplicated/stranded by insert+move on `main` too | Journal gap is this stack's; the stranding is upstream |
| 20, 23 | `debug_assert` panic "Larger-than-1x1 array reached scalar-context cell" when `=NAME` is set before the name exists and the name is then defined as a range | Reproduces on `upstream/main` (`model.rs:1082`) | Pre-existing; report upstream |
| 5, 126, 53 | `set_user_input` into a CSE **member** is accepted; Full re-fills it from the anchor next pass (user input silently lost), Incremental keeps it | Pre-existing (`main` identical). Fix: reject writes into CSE members | Upstream; Incremental is the correct side |
| 7, 66, 99 | After insert/delete cols/rows, `move_cell` drops CSE member `SpillCell`s; the anchor's re-fill errs (`mod.rs:1131-1136` `.ok_or("expected a column")`) so readers of the anchor see `#ERROR!` forever in Full | Pre-existing (`main` identical). One-line fix: re-fill via `worksheet.update_cell` like the dynamic branch | Upstream; fix in #90 anyway or the PR's own long fuzz stays red |
| 142 | Freshly-set CSE anchor is not in phase-1 `spill_cells`; a reader of a member evaluates before the anchor fills it | Pre-existing. Fix: include CSE anchors in `collect_spill_cells` | Upstream |
| 32, 67 | A `Set ""` on an already-empty formula cell, or an `OFFSET` seed, is in the delta with no observable change | Content did change (formula removed); the harness's `value_seed()` ignores `""` sets | Harness strictness; document the contract |

Also carried from this pass: CI's fuzz job must stop excluding structural ops (`build-test.yaml:52`) or it
cannot catch any of the above; `WriteLog` is `pub use`d from `lib.rs` (intended?); `Input::ColHidden` is
`#[allow(dead_code)]` (column hiding is not an input to anything yet — confirm SUBTOTAL semantics).

## 9. Known residuals (do not re-report unless you disagree)

- Incremental/Verify may re-associate multi-column `SUM` (documented on `RecalcMode::Incremental`); default
  Full does not.
- Defined names are not displaced on insert/delete — pre-existing `main` behaviour; the graph models it.
- A dynamic array in the cone, or created by the pass, falls back to Full.
- CSE arrays + `insert_columns` nondeterminism is pre-existing on `main`.
- Python/Node bindings are Full-only.
- CI fuzz excludes structural ops (see §5).

## 9b. Per-PR readiness at these heads (my assessment; form your own)

| PR | Score | Blocking | One change that flips it |
|---|---|---|---|
| #90 | **5/10** — architecture is now right; two disqualifying defects remain | D1 (Excel/main semantics + xlsx), D2 (unbounded per-pass growth), D3 (CSE guard) | Replace `FormulaValue::Empty` with coercion-at-the-boundary (D1); D2 and D3 are each ≤10 lines |
| #93 | **7/10** — generation stamps sound, bit-identical to main on both archs in Full | none | Compose in per-cell scan order so Incremental `SUM` is bit-identical to Full, then drop the harness tolerance |
| #94 | **9/10** — zero fast-vs-string divergences across a 40-shape × 7-script matrix; journal and edge shifting verified | none | A test pinning the `ImplicitIntersection{automatic}` flag behaviour on xlsx export after a structural edit |

Stack hygiene verified: #94 is a zero-line diff from #90 outside `displace.rs`/`stringify.rs`/`actions.rs`;
#93 adds only `range_reduce.rs` and its hooks; no static-system leftovers in any tree. `WriteLog` is
`#[doc(hidden)] pub` because the `write_log` field is public — acceptable, but say so in the PR.

## 10. Verdict format

For each PR: **Approve / Not yet**, plus (1) confirmed bugs with `file:line`, a ≤6-op repro, and class from
§7; (2) any default-mode difference from `main`; (3) the one change that would flip your verdict. Put it on
the PR, not in chat.

---

## 11. Status update (2026-08-24) — heads re-verified after the D-fix round

Heads at this update: #90 `f75148ad` (pushed), #93 `64bc24a0` (local merge of #90, **unpushed**),
#94 `fc6ce848` (local merge of #90, **unpushed**). Worktrees: `/private/tmp/fix{90,93,94}`.

Resolved since §8b (verify with `git log e448d15d..f75148ad`):

- **D1 fixed as prescribed**: `FormulaValue::Empty` removed; blank formula results coerce to `Number(0.0)`
  at the formula-result boundary (`model/mod.rs` ~1771 and the array branches); xlsx export/compare reverted
  to `main` behaviour. `probe_empty.rs` now asserts the main-compatible values.
- **D2 fixed**: `SheetRanges::remove_dependent` prunes the `bands`/`wide` index entry when a range loses its
  last dependent; regression test `removing_last_dependent_prunes_the_index`.
- **D3 fixed**: `array_footprint` inserts every anchor's full `r` rectangle; `replace_arrays` rebuilds the
  index at graph build (no per-pass walk — the single call site is the non-Full branch of `evaluate_full`).
- Link writes journaled (`Write::Link`, incl. range clears); 1×1 dynamic anchors stay incremental;
  per-pass scratch (`support`/`lambdas`/`variable_stack`) cleared; CI fuzz no longer sets `FUZZ_AVOID_OPS`
  (structural ops now covered in CI).
- **#93 should-fix done**: composed aggregates fold in row-major scan order — bit-identical to Full — and the
  `RecalcMode::Incremental` doc now says so; `probe_sum_association.rs` pins the cancellation shape (`4.1`).
  New `ReferencedRanges` in `dependency_graph.rs` is shifted on structural edits.
- Process items done: #93 retargeted to `incremental-recalc-fanout-guard` on GitHub; all three PR
  descriptions describe the three-PR stack with #89 folded in.

Verification at these heads (all in the fix worktrees, 2026-08-24): default Full / forced Incremental /
Verify suites green (2367 lib tests each) on all three tips; clippy `--all-targets --all-features -D warnings`
clean; xlsx crate green (#90); structural-inclusive fuzz green — #90 at 60×200 plus Verify-mode 20×120,
children at 40×150 — no divergences, no FLAKY tags.

Remaining:
- Push `incremental-recalc-range-composition` (`64bc24a0`) and `incremental-recalc-lazy-crud` (`fc6ce848`)
  so the PRs contain the D-fix round.
- Nit: `lib.rs` has a duplicated `#[doc(hidden)]` on the `WriteLog` re-export.
- x86_64 (Rosetta) lib suite + wasm `make tests` at `f75148ad`: run recorded in
  `/private/tmp/fix90/extra_results.log`.

## 12. Quality gates and how each is checked (2026-08-24)

**Correctness (Incremental ≡ Full)** — three independent mechanisms, all green at the pushed heads:
Verify oracle (2367 tests forced through Incremental+assert+Full-replay), structural-inclusive
differential fuzz (#90 60×200 + Verify 20×120; children 40×150), and the 21 pinned regression tests from §7.

**No regressions vs `main`** — the entire upstream suite runs unchanged in default Full (2367, incl.
x86_64). Audited that the stack did not move the goalposts: vs `origin/main` only 9 test files are touched,
none change an upstream expectation (formatting-equivalent pattern rewrites + ~2,000 new test lines only).
§6.3 bit-identity items each have dedicated tests; D1's fix removed the one real default-mode divergence.

**Idiomatic Rust (bar: `~/codex/codex-rs` workspace lints)** — differential clippy with codex-rs's 35-lint
deny-list (as `-W`), run on `origin/main` vs each tip (`lint_codex.sh` in the fix90 worktree):
- main: 111 findings (57 `uninlined_format_args`, 41 `redundant_closure_for_method_calls`,
  11 `redundant_clone`, 2 `trivially_copy_pass_by_ref`) — all pre-existing upstream code.
- #90: 111, the identical set (only the `model.rs`→`model/mod.rs` rename). **Zero added** across ~5k new
  lines; no `unwrap`/`expect` outside tests; new test files carry
  `#![allow(clippy::unwrap_used, clippy::expect_used)]`, matching codex-rs's tests-exempt convention.
- #93: 111 identical. #94: **110** — removes one pre-existing `redundant_closure_for_method_calls`
  in `actions.rs`, adds none.
- Deliberate non-alignment: codex-rs rustfmt (`imports_granularity="Item"`, edition 2024) is not applied —
  PRs into IronCalc follow IronCalc's own fmt/clippy gate, which is clean at `-D warnings`.

## 13. Final adversarial round (2026-08-24, heads f75148ad / 64bc24a0 / fc6ce848) — verdict

Independently verified at these heads (fresh worktrees, not the fix worktrees): all three modes green
(2367/2384/2377), tracked probe tests green, structural-inclusive fuzz **100×200 zero failing seeds** plus
Verify-mode 40×150 on every tip, x86_64 suite green, clippy clean under the CI flags, xlsx + wasm green.
D1 re-verified against `main` cell-by-cell (identical) and the xlsx round-trip writes `<v>0</v>` again.
The #93 scan-order rewrite is bit-identical by construction (cached prefixes are the direct scan's own
intermediate accumulators; a non-identity accumulator streams and forgoes the cache, with a `to_bits`
check that handles the `-0.0` corner), and running totals stay O(n) (5 ms vs 127 ms at n=2000).

Adversarial delta review findings, resolved:

- A claimed `test_to_from_bytes::errors` failure **does not reproduce** on a clean tree (agent-local state).
  Optional hardening: assert only the `"Error parsing workbook:"` prefix. Positive side-finding worth
  keeping: `.ic` bytes are wire-compatible with `main` in both directions.
- **CSE first-evaluate transient (pre-existing, both modes, also on `main`)**: `A1==SEQUENCE(2,1,C2)` +
  CSE `{7;20}` at C1:C2 → first evaluate `#VALUE!`, second `20`, identically in Full and Incremental. Not a
  divergence and not a regression; `761adfee`'s message overclaims closing this class (phase-1 CSE anchors
  cannot be reordered because `get_spill_area` returns empty for CSE). One-line upstream follow-up:
  return the CSE rectangle from `get_spill_area`.
- Stale docs to fix in-place: `base/src/recalc/README.md` still says "FormulaValue::Empty preserves blank
  results" (the variant is deleted; the design is coerce-to-0); `incremental.rs` ~265 references the deleted
  variant; #90's `RecalcMode::Incremental` doc mentions re-association that #93 has since removed; the
  fallback list still says "a dynamic array … among the affected cells" though 1×1 anchors stay incremental.
- Nits: `probe_empty.rs` prints without asserting (add asserts); `Write::Link` stores the sheet index at
  push time while `Write::Cell` re-attributes at drain (benign only because sheet CRUD forces full — add a
  comment or align); link journal pushes on no-op link writes (spurious seed-exempt delta entries);
  duplicated `#[doc(hidden)]` on the `WriteLog` re-export.

Intentional, verified semantic changes vs `main` (all match Excel; list them in the #90 description):
reader-above-source of a blank pass-through now sees `0` consistently (main was order-dependent);
`=NAME` typed before the name exists then defined as a range applies legacy implicit intersection instead
of a debug panic / `#VALUE!`; writes into CSE member positions are rejected ("can't change part of an
array") instead of silently overwritten.

**Verdict: #90 approve with the doc fixes above (9/10) · #93 approve, rebase after #90's doc commit
(9/10) · #94 approve (9/10).** No known incremental≠full divergence, no default-mode difference vs
`main` beyond the three intentional Excel-aligned changes, no open crashes, flat per-pass cost, and three
independent oracles green with structural ops included. The remaining items are documentation and one
pre-existing upstream quirk — none blocks merge.

## 13. Adversarial review round (2026-08-24) — executed per RECALC_REVIEW.md

29-agent workflow (6 mechanism finders → 15 seeded mutants → 2-refuter panels) + a 400×400 differential
soak (131k evaluates) + an Opus triage/fix round. Heads before: #90 `f75148ad`. Heads after (local,
unpushed): #90 `b461fff6`, #93 `4f865165`, #94 `d1249e26`.

### Introduced bugs found and fixed (all refuter-confirmed, all with named regression tests)

1. **Incremental one convergence pass behind Full** (6 soak seeds: 84/157/170/251/301/386; spill/CSE debt
   after structural edits, cycles closed by newly-observed edges, frontier double-evaluating a mid-cycle
   cell). Fixed by a `convergence_debt` signal on the graph (a footprint cell that moved during an
   incremental-initiated full pass and was read forces the next pass full) plus a `saw_circular_reference`
   flag (a `#CIRC!` in a cycle the graph did not contain redoes the pass full — Full's placement by
   construction). Full-mode semantics untouched; bench flat (118×). 7 regression tests; the 400×400 soak
   that found the class is now clean.
2. **Default-mode drift: row/column moves rejected when a CSE anchor is in the moved line**
   (`covered_by_cse_rect` guard not suspended in `move_column_unchecked`/`move_row_unchecked`; column case
   nondeterministic via HashMap order, row case failed deterministically). Fixed like `move_cell` +
   `column_cell_references` now sorted. Zero result-mismatches in a 400-seed sweep after.
3. **`range_clear_all` deleted styles of spill cells outside the cleared range** (rewrite used
   `remove_cell` on the whole footprint; baseline used `cell_clear_contents`). Restored baseline semantics,
   byte-identical differential probe vs baseline; link journaling preserved.

### Oracle holes found by mutation testing (15 mutants: 10 killed, 3 holes closed, 2 equivalents)

- M2 wide-rect trace loss: only sheet-dimension-clipped (whole-column/row) refs expose it — 3 killing tests.
- M8 volatiles never re-firing: Verify structurally excused the always-dirty set and the fuzz zoo had zero
  volatiles. Fixed with a Verify **liveness assertion** (every always-dirty cell must be in the delta),
  3 lib tests, and value-deterministic volatile zoo entries (`=RAND()*0`, `=RANDBETWEEN(1,1)`, …) planted
  on every 5th seed (delta-membership check `volatile-not-reported`).
- M11 displace flag-swap on equal resolved endpoints: text-level killing tests (verified against #94).
- M12: reachable only via xlsx import (`=LET(x,A6:A8,x)` survives as plain non-Scalar) — new xlsx-crate
  test; failure mode is a silent `#VALUE!`, not the debug panic the guard's comment claims.
- M14 (`b.min(a)` flip) proven semantically equivalent (NaN/±0-symmetric) — no information.

### Intentional divergences to document in PR bodies (confirmed intentional, do NOT fix)

- #90: blank-result coercion makes same-pass readers of a blank-returning formula see `0` where baseline
  was **order-dependent** (readers before the blank formula in eval order saw Empty, after saw 0). New
  behavior is deterministic, Excel-consistent, stored-value-consistent. Needs a PR-body paragraph.
- #94: refs displaced past LAST_ROW become `#REF!` (baseline: out-of-range text evaluating `#NAME?`).
  Excel-consistent, matches the column-overflow behavior baseline already had. Needs a PR-body line.

### Post-merge verification at `b461fff6`

Full/Incremental/Verify suites, clippy `-D warnings`, fmt, xlsx crate, x86_64 lib, wasm — green.
Soak 400×400: `failing_seeds=[]` (133,653 evaluates). Verify-fuzz 60×200 with the liveness assertion: clean.
Residual (documented, sound): ~1.5pp more `Everything` deltas from debt-forced full passes; arrays that
move and are read pay one extra healing full pass; workbooks without arrays unaffected.

## 14. Round 2 (2026-08-24): mutation-testing the fixes themselves

10-agent workflow: 9 mutants against the round-1 fix mechanisms + 3 adversarial diff reviewers + refuters,
plus a fresh 400×400 soak on unseen seeds 401–800 (clean — the fixes generalize). Heads after: #90
`f7f7ffcf`, #93 `ae2724a5`, #94 `c2842a83`.

**Mutation:** 8/8 real mutants killed at stage 1 by the round-1 regression tests; the debt condition's two
arms have disjoint killing tests (independently load-bearing). N9 (remove the Verify liveness assertion)
survives as expected for an oracle-weakening mutant — documented, with its victims shown redundantly covered.

**Heuristic falsification:** four attack angles on the convergence-debt condition (cascaded debt, invisible
readers via names/INDIRECT/range edges, the Unevaluated-anchor exclusion, #93 cache across debt/redo passes)
found no gap. Enforcement gates added (`ed559817`): grep-gate forcing `*_unchecked` rebuild paths to suspend
the CSE guard, grep-gate + behavior pin on footprint sweeps, rebuild-order determinism pin.

**New findings, all fixed:**
- F1 (major, residual #90 gap, predates round 1): known-cycle `#CIRC!` placement diverges through a 1×1
  dynamic anchor — `recompute_all` was row-major but Full evaluates ArrayFormula anchors phase-1 first.
  Fixed by walking the known-cycle cone in Full's two phases (`3b19dc81`); acyclic path proven unaffected
  (topo order) with a positional-counterexample test.
- F2 (major): the new Verify liveness assertion false-positived when a branch flips a formula INTO
  volatility with unchanged value (post-pass set vs pre-pass seeding). Fixed by asserting against the
  pre-pass snapshot (`9a587551`); leave-transition and steady state still mutation-verified.
- F3 (minor): volatile seed-planting tripped the fuzzer's own coverage floor at 40×40; floor now computed
  over non-volatile seeds with a forced-Everything negative test (`ef028a0e`).
- F4 (minor): one `needless_collect` — deliberately kept as a by-reference snapshot (fusing it would have
  sampled change_key after the mutation, inverting the debt check); codex lint count back to baseline 111.

**Refuted correctly:** phantom `EmptyCell`s after clearing a whole spill — pre-existing baseline behavior
(`main` uses the identical pattern); the styles fix restored baseline exactly, phantoms included.

**Verification at `f7f7ffcf`:** full matrix green including BOTH soak windows (seeds 1–800 × 400,
267k evaluates, `failing_seeds=[]`), Verify-fuzz, x86_64, wasm, clippy/fmt, codex deny-list at baseline.

**Why the loop cannot be replaced by construction (recorded from discussion):** the invariant is
equivalence with Full, and Full is a program with emergent, accidental order-properties, not a written
spec. The by-construction mechanisms (tracer, journal) produced zero findings across both rounds; every bug
lived in the scheduler (which must mirror Full's accidents) or duplicated conventions (now grep-gated).
Structural directions if pursued upstream: make Full a one-pass fixed point (kills the debt class as dead
code), whitelist selectivity instead of blacklisting hazards (missed cases degrade to perf, not wrongness),
enforce oracle/mechanism accessor independence, wire soak + mutants into nightly CI, add coverage-guided
fuzzing over the differential harness.

## 15. Codex-rs-style enforcement alignment (round 4, in progress)

Per-invariant mapping to /Users/brian/codex/codex-rs idiom (their observed patterns: clippy.toml
disallowed-methods with reason strings; #[must_use] Drop guards like TurnProfileTimingGuard; newtype ids;
98-crate workspace with privacy at crate boundaries). Implemented on `recalc-r4-codex-align`:
footprint primitive ban via disallowed-methods (replaces grep-gate where strictly stronger); recording
pause as a #[must_use] Drop guard with raw set_recording privatized (closes the one legal journal-bypass
window structurally); CSE guard flag private-by-module (bypass no longer compiles); traced-read ban if
cleanly expressible.

Deliberate FOLLOW-UPS, not in this stack (each is a churny refactor that would hurt reviewability):
1. `SheetIndex` newtype / generation-stamped sheet ids — makes stale Position across sheet renumbering
   unrepresentable (today: `invalidate_graph` convention). Codex pattern: ResponseItemId-style newtypes.
2. Split `ironcalc_recalc` into its own crate (trace/journal/graph/scheduler behind a narrow API) — codex
   organizes by crate-per-concern (98 crates); privacy at crate boundaries beats pub(crate) discipline.
3. Adopt the codex workspace deny-lint list in [workspace.lints.clippy] — requires fixing ~111 pre-existing
   upstream findings first (mostly uninlined_format_args); a repo-policy decision for the maintainer.
   Until then the differential lint script (lint_codex.sh) is the gate: new code must add zero findings.

Fundamental limit (restated): the core invariant — Incremental ≡ Full — is a relational property across two
executions and cannot live in the type system at any rung; it stays enforced by the Verify oracle,
differential fuzzing (now coverage-guided), and mutation-proofs, run continuously.

## 16. Round 4 complete (2026-08-25): the structural cycle rule + codex-rs enforcement

**The cycle rule** (3 commits ending `1832e49d`): a cell serves its stored value only when that value is a
function of its inputs. `DependencyGraph::never_served` = cycle members + everything downstream + any cell
whose stored value is `#CIRC!` (the evaluator's own witness, surviving unmodelled reads); seeded dirty every
pass. `blocked_array_readers` = readers of `#SPILL!` anchors; cone reaching one falls back to full (scoped —
a blunter always-dirty version tripped the delta-coverage floor at 37%). Verify's stored-vs-live skip list
IS the never-served list — one list, fixing seed 251's under-skip (cells downstream of cycles). Enabling
change: footprint→anchor read edges, so cycles closing through array members are visible to the graph.
Notable: the agent deleted the anchor phase on a sound argument, the corpus fuzz falsified it in one run
(cycle closing around a not-yet-never-served anchor), it restored the phase and pinned the shape. M5/M7
retained despite unkilled mutants, with stated defense-in-depth reasons. All 8 covfuzz shapes fixed +
seed 251; 15-min covfuzz re-run 0 artifacts; bench +1.5% (noise).

**Codex-rs enforcement alignment** (5 commits, merged): workspace `clippy.toml` `disallowed-methods` ban on
`remove_cell` (8 sanctioned allow-sites, one documented shape; NOTE clippy matches the *declaration* path —
`ironcalc_base::types::Worksheet::remove_cell`, not `worksheet::`); recording pause is a `#[must_use]`
Deref-holding Drop guard with `set_recording` private to `mod journal`; CSE flag private in
`model/cse_guard.rs` (bypass = E0616); two grep-gate clauses deleted as strictly-superseded, positive
obligations kept. Untraced-read ban NOT forced: raw getters proved already privacy-unreachable from
`functions/`; remaining candidates need 57–687 allow-sites. Real fix recorded on the gate: `TracedModel`
newtype receiver for `functions/` (follow-up, would delete the gate outright).
