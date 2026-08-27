# Incremental recalculation

Incremental recalculation recomputes only the cells an edit affects, instead of the whole workbook. It is opt-in through `RecalcMode::Incremental`. The default stays full recalculation. Anything the incremental path does not model falls back to a full pass, so results always match full recalculation.

The engine records every write as it happens, observes what each formula reads while it evaluates, and uses the resulting dependency graph to recompute just the affected cells in dependency order. Recalculation stops early wherever a recomputed value turns out unchanged.

```mermaid
flowchart LR
    E[User edit] --> J[Write journal]
    J -->|evaluate| D[Affected cells]
    G[Dependency graph] --> D
    D --> O[Evaluate in dependency order]
    O -->|value unchanged| S[Stop early]
    O -->|reads observed while evaluating| G
```

The design follows the same pattern as reactive UI libraries such as MobX, Vue, and signals. Dependencies come from watching real reads, and invalidation is pushed through the graph until an equality check stops it. The one difference is that recomputation is eager rather than lazy. Every cell of a spreadsheet is visible output, so deferring work never pays.

## How a pass works

`Model::evaluate` in incremental mode (`model/incremental.rs::evaluate_selective`):

1. Drain each worksheet's write log and derive the dirty set from it. A cell that stopped being a formula also drops its outgoing edges.
2. Add every cell that reads the clock or a random source, and every cell whose last result was not a function value (see below). Those are always dirty.
3. Collect every cell reachable from the dirty set through cell, range, and input edges.
4. If the pass cannot be modeled (see fallbacks below), run a full pass instead.
5. Evaluate the affected cells in topological order. While each formula runs, a tracer records what it reads; when it finishes, those reads replace its edges in the graph.
6. If a recomputed cell's value, type, and link are unchanged, nothing downstream of it is recomputed.
7. Cells outside the affected set are served their stored values. A formula whose live result is an empty cell coerces to `Number(0.0)` at the result boundary, before the value is stored — `FormulaValue` has no blank variant. So the stored value is exactly what a live read of that cell returns. That coercion is what makes the full pass itself order-independent (a same-pass reader sees the stored `0` whether it ran before or after the cell it reads), and it is the same reason serving a stored value never changes a result here — for every cell whose stored value is a function value.
8. Changes are accumulated for `Model::take_changed_cells`.

## Where things live

| File | Role |
|---|---|
| `recalc/journal.rs` | `Write` and `WriteLog`. Worksheet mutators push; `Model::evaluate` drains. Evaluation writes (storing a formula's result) are not journaled, because they are not edits. |
| `recalc/trace.rs` | `ReadSet` and `Input`. Records the cells, rectangles, and non-cell inputs one formula reads. A covering rectangle suppresses per-cell edges, so `SUM(A:A)` stays one edge. |
| `dependency_graph.rs` | The graph itself: edges keyed by cell, range, and input, a banded range index (`SheetRanges`), `replace_reads`, `reachable`, `topo_order`, `structural_edit`, and `RecalcMode`. A structural edit shifts every index through `Shift`, applied field by field in `shift`, which destructures the struct so a new index cannot skip it. |
| `model/incremental.rs` | The scheduler: `evaluate_selective`, the fallback decisions, and the frontier and whole-cone recomputes. The *scheduling* decision lives here and only here; the evaluator's own mode branches are the `tracing()` gates in `model/mod.rs` and the dispatch in `Model::evaluate`. |
| `model/changed_cells.rs` | What counts as an observable change (`ChangeKey`), and the delta `take_changed_cells` reports. |
| `model/array_index.rs` | `array_footprint` and the walks that maintain the array/spill index and the formula count between full passes. |
| `model/unstable_cells.rs` | Rebuilds one of the two sets of cells whose stored value may not be served (below): the readers of blocked spill anchors, which is the set that needs stored cell state the graph cannot see. The other set, the cycle cone, is derived from edges alone, so the graph computes it itself (`cycle_cone`) and each scheduler installs it after its pass. |
| `model/cse_guard.rs` | The CSE member guard flag and the only scope that may suspend it. |
| `model/verify.rs` | The `RecalcMode::Verify` oracle. Compiled only under the `recalc_verify` feature. |
| `worksheet.rs` | The only producer of journal entries. `sheet_data` is written through mutators that push a `Write`. |
| `model/mod.rs` | `evaluate_cell` pushes a `ReadSet` frame, the `trace_cell`/`trace_rect`/`trace_input` helpers record into it, and a finished formula commits its reads to the graph. Tracing runs only in incremental mode. Also `evaluate_full`, whose two-phase order the incremental path must reproduce: `is_phase_one_cell` and `cells_in_order` are that order's one definition. |

## Cells that never serve a stored value

Serving a stored value is only sound when that value is a function of the cell's inputs. Two kinds of cell fail that test, and both are found by reading the state the last pass left rather than by matching a shape:

- **A cell on a dependency cycle and anything downstream of one.** A cycle has no fixed point: what its members hold is an artifact of which member the walk entered first, and a reader of one holds whatever it saw mid-cycle (`COUNT` swallows a `#CIRC!` into a number). Full re-derives all of it from scratch on every pass, so incremental seeds those cells dirty on every pass and recomputes them and their readers. The set is the cells `topo_order` cannot place (those on a cycle plus everything after one), rebuilt over the whole graph after each full pass and over the cone after each incremental one.

  The set used to carry a second half: every cell whose stored value was `#CIRC!` — the evaluator's own report that it re-entered itself — kept in case some read that closed a loop left no edge behind. There is no such read. `evaluate_cell` records the read before any early return, including the one the incremental scope answers from the store, and a formula commits its reads on both exits, so every cross-cell read that can re-enter is an edge. The one edge the graph drops on purpose is a cell's read of *itself* (`replace_reads`: `if p != dependent`), and that is exactly what the witness half added: self-references, and the `#CIRC!` that propagates out of one. A self-cycle has a single entry point, so what it holds *is* a function of the cell's inputs, and every non-self read it makes is still an edge, so anything that could move its value dirties it the ordinary way. The half is gone: it never changed a result, and while it stood it left every self-referential cell permanently dirty — and, for one inside an array footprint, every later pass full.
- **A reader of a blocked spill anchor.** The anchor stores `#SPILL!` but hands a same-pass reader the live array's top-left value, so a reader recomputed against the stored error gets something a full pass never produces. Their stored values are served — they are what full computed — but recomputing one takes a full pass, which is the only pass that evaluates the anchor live, so a cone that reaches one falls back.

`RecalcMode::Verify`'s stored-vs-live check skips exactly these cells, because they are exactly the cells a one-cell scratch frame reading the store cannot reproduce. The two lists are the same list.

Cycle cells are recomputed on every pass but are not *reported* on every pass: the delta still names only the cells whose observable state moved. Full's `#CIRC!` placement can shift with any edit, and when it does the recompute sees it and the delta says so.

## Array footprints are edges

An array anchor writes its spill members as evaluation writes, not edits, so nothing journals them. Reading a member is therefore recorded as a read of the anchor: the array index maps every footprint position to its anchor, and `evaluate_cell` records the anchor's position along with the position actually read — including a position whose spill cell a structural edit dropped, whose index entry survives until the next full pass refills it, and including reads that the incremental scope answers from the store. Without that edge a cycle running through an array footprint is invisible to the graph, and the cells around it look like ordinary results.

The index holds each anchor and each spill cell, not each anchor's declared rectangle. A CSE anchor owns a rectangle it refills whether or not the cells are there, but a *ghost* member — a declared position with no spill cell — only exists between the write that created it and the anchor's next evaluation, and that write dirties the anchor. The anchor is in the index, so the cone holding it goes full, and that is the pass that refills the rectangle. Indexing the rectangle restated what the anchor's own entry already said.

## Non-cell inputs

Volatility is an input, not a list of functions. `NOW` records `Input::Clock`, `RAND` records `Input::Random`, `SUBTOTAL` records row visibility, `ROW()` records its own position, and `OFFSET`/`INDIRECT` record their resolved targets plus `Input::Computed` so structural edits re-run them instead of shifting a stale snapshot. Readers of `Clock` and `Random` are always recalculated. Whether a cell must be recomputed and whether it belongs in the change report are separate facts, so a deterministic formula next to a volatile one is never reported by mistake.

## When the engine falls back to a full pass

- The graph is not ready: the first evaluation, or after sheet add/delete/rename, defined-name changes, locale, or timezone.
- Row or column moves. Inserts and deletes stay incremental; the graph shifts positions and edges in place.
- A dynamic array or spill anchor is among the affected cells. Spills need the full pass's two-phase ordering.
- The edit reaches more than half the workbook's formulas (with a floor of 1024, so small workbooks never fall back). This one is a performance choice, not a correctness one, and Verify disables it.
- The pass reported `#CIRC!` for a cycle the graph did not already contain. The closing edge is only observed while the pass runs, so the cone was ordered without it and the error would land on a different cell than the full pass picks. A cycle the graph already knows about is walked by position instead, in the full pass's own two phases: array formulas first, then everything else, each row-major. That is the order the full pass walks in, and the cone contains every cell full could reach a cycle member through, because such a cell reads one transitively and so is a reader of an always-dirty cell. Since a known cycle is in every cone, this is also the walk a cycle *closing* on this pass gets when another cycle is already open — which is why phase 1 stays: the pass an anchor first falls inside a cycle, the anchor is not yet a seed, and only phase 1 makes full enter the cycle where it does.
- The pass itself wrote into an array footprint: a spill landed, a CSE range filled, or an anchor stored `#SPILL!`. This one is only visible after the fact — the pre-pass check above catches an anchor that is *already* in the array index, and this catches an affected cell that turns into one while the pass runs (a scalar-result anchor whose result grows, say). The pass is redone as full, so spill dependents are not missed and `collect_array_cells` rebuilds the index exactly rather than patching it.
- The cone reaches a reader of a blocked spill anchor.
- The previous pass left convergence debt (see below).

## Convergence debt

A full pass is not a fixed point. Its phase 1 spills arrays and its phase 2 evaluates the rest, so a formula can read a spill member before the anchor refills it, and a cycle that runs through an array member resolves against that member's stored value. Full recalculation heals those readers on its *next* pass, because it rescans everything unconditionally.

Incremental has to match that pass for pass. So a full pass run from the incremental scheduler compares the array footprint's values across the pass: if a footprint cell moved and something read it, the pass left debt, and the graph records it so the next pass is full too. The debt clears itself, because the healing pass moves nothing and the pass after it is selective again. A workbook with no arrays never records debt, so plain editing is unaffected.

The condition used to have a second arm: a cycle running through the array (a member read while its anchor was still evaluating) also counted as debt, on the grounds that such a read leaves no edge behind. That arm could not decide anything, and it is gone. Reading a footprint member records an edge on its anchor (I1.9), so a cycle through a footprint puts the anchor on the cycle; the anchor therefore lands in `never_served`, which every later pass seeds dirty; and a cone holding an array position takes the arrays→Full fallback. The next pass was already full, which is the whole of what the debt flag would have forced.

## Design rules

- No *write* reaches the graph except through the journal. `model/mod.rs` dirties the graph only from `drain_write_journal`, and the mutation paths (`actions.rs`, `undo_redo.rs`, `clipboard.rs`, `common.rs`) never touch it at all — both halves are held by the `graph_is_only_notified_by_the_journal` gate. The scheduler's own `mark_dirty` calls are not writes: they seed the pass with the always-dirty cells, the never-served cells, and the array anchors a full pass first observed, none of which any edit reports.
- Edges are the reads observed at evaluation time. There is no static analysis of formula text.
- A cell's value is a function of its inputs only. Modes choose which cells to evaluate, never what they evaluate to.
- Anything the model cannot represent falls back to full recalculation rather than approximating.

## Testing

```bash
# Full suite, default full recalculation
cargo test -p ironcalc_base --lib

# Full suite with incremental recalculation forced on
IRONCALC_RECALC=incremental cargo test -p ironcalc_base --lib

# Same suite plus a shadow full-recalculation pass that asserts both agree
IRONCALC_RECALC=verify cargo test -p ironcalc_base --features recalc_verify --lib

# Randomized comparison of both modes in lockstep, as run in CI
cargo test -p ironcalc_base --test fuzz_differential -- --nocapture

# Benchmark: cost of a single edit, incremental vs full
cargo test -p ironcalc_base bench_incremental --release -- --ignored --nocapture
```

`RecalcMode::Verify` (behind the `recalc_verify` feature) runs the incremental pass, asserts that the change report lists every change and nothing else, asserts every stored formula value equals a live re-evaluation, then runs a full pass on a snapshot and compares, so the check cannot repair the state it is checking.

## Invariants and their witnesses

The suite here is large next to the engine, and the only thing that makes that defensible is that every test is the minimal witness of a named clause. This section is the map: eight invariants, the clauses each decomposes into, what enforces each clause, and — where the answer is "a test" — which test.

Four things can enforce a clause, and only two of them owe a test:

| | What it means | Owes a test |
|---|---|---|
| **construction** | The type system, a privacy boundary, or the shape of the code makes the violation unrepresentable. | No. A test here asserts what the compiler already says. |
| **gate** | A grep-gate in `test/test_recalc_invariants.rs`, or a `clippy.toml` `disallowed-methods` entry. | No — the gate is the witness. |
| **test** | Nothing but a test sees it. | Yes: one witness, named below. |
| **oracle** | `RecalcMode::Verify` and the differential fuzzer find it over shapes nobody enumerated. | Only a deterministic fast gate, and only where it kills something no kept test kills. |

`base/tests/common/` — the generator, the lockstep harness, the minimizer — is the mechanism behind the **oracle** column. It is the largest file in the suite and it is tooling, not tests. It plants six SUMIFS shapes, eleven OFFSET/INDIRECT shapes, seven SUBTOTAL shapes and a volatile zoo on every run, which is why one deterministic witness per clause is enough here and a second shape of the same clause is not.

### I1 — every evaluation read records an edge or an Input

| Clause | Enforcement | Witness |
|---|---|---|
| I1.1 A cell read is recorded before any early return — the scope gate that answers from the store, and the `#CIRC!` return | construction (`trace_cell` is the first statement of `evaluate_cell`) | — |
| I1.2 A function implementation reaches cell state only through the recording accessors | construction (`functions/` runs on `EvalCtx`, a newtype over `&mut Model` with a private inner reference; it has no `workbook` field and no untraced getter, so the bypass does not compile) | — |
| I1.3 A rectangle is recorded at its declared extent, not the extent the walk visited: `SUM(B:D)` clips its per-cell walk to the used range, so only the rect connects a write outside it | test | `multi_column_range_edits_propagate` (three columns wide, so it kills both a >1 and a >=3 rect drop; the old two-column shape pinned only the first); the run-time-widened case is `incremental_sumifs_reads_resized_criteria`; the `#` operator's edge on a not-yet-written anchor is `spill_hash_of_empty_anchor_sees_later_formula` |
| I1.4 Each volatile or environment function records its `Input` | test, one per variant | Random: `volatile_rerolls_across_repeated_incremental_passes`. Clock: `clock_volatile_is_reported_on_every_incremental_pass`. OwnCoord: `incremental_argless_row_updates_after_insert` |
| I1.5 Reading a cell's formula text or formula-ness records `FormulaText` — three independent call sites | construction (all three ways in — `EvalCtx::get_cell_formula`, `get_english_cell_formula`, `formula_index_at` — record it themselves) | — |
| I1.6 Reading row visibility records `RowHidden`, keyed per row | *that* it records: construction (`EvalCtx::row_hidden` is the only way in and traces first). *Which row* it keys on: test | `subtotal_sees_hidden_row_it_scans_not_own_row` |
| I1.7 Reading a defined name records `Name` | test | `name_reader_redirty_on_insert` |
| I1.8 A reference-returning function's resolved target becomes an edge, at the extent it resolved to | test; the extent is one per call site | `offset_target_change_without_static_edge`; the extent is `offset_records_its_resolved_extent_not_the_walk` and `indirect_records_its_resolved_extent_not_the_walk` (I1.3's clipping rule, reached through a computed extent) |
| I1.9 Reading an array-footprint position records an edge on its anchor, taken from the array index and recorded ahead of the scope gate | test | `cse_footprint_cycle_stored_value_diverges_from_a_live_reeval` (dies only under `recalc_verify`) |
| I1.10 A formula commits its reads on both exits | construction | — |

Two reads in `functions/` used to bypass the recording accessors — the
financial whole-column clip and `SHEET(a_defined_name)`. Both now go through
`EvalCtx::sheet_dimension` and `EvalCtx::defined_name`, so I1.2 has no
exceptions. Neither was ever a wrong answer, and neither has a witness,
because neither *can* be one:

- the clip's declared rect is recorded whole when the `RangeKind` node is
  evaluated, before any function clips its walk (I1.3), so the rect already
  covers every write that could move the result. Delete that `trace_rect` and
  a whole-column `NPV` does go stale — which is I1.3's witness, not a new one.
- every defined-name edit calls `invalidate_graph`, so a name reader is never
  served from the store at all.

A test asserting either would pass before the fix as readily as after. They
are routed for one reason only: so that "every accessor on `EvalCtx` records"
has no exceptions to remember.

### I2 — every user mutation journals; the pause is guard-scoped

| Clause | Enforcement | Witness |
|---|---|---|
| I2.1 A no-op write and a style-only materialization are not edits; a write that changes state is one even when the formula index does not move | test | `style_on_blank_cell_does_not_enter_the_delta`, `blocked_spill_reset_on_insert_below_matches_full` |
| I2.2 Nothing outside the journal drain dirties the graph | gate | `graph_is_only_notified_by_the_journal` |
| I2.3 Recording is paused only through a `#[must_use]` Drop guard | construction (`set_recording` is private to `mod journal`) | — |
| I2.4 A displacement journals a value write substituted for the suppressed formula write | test | `incremental_handles_row_delete` |
| I2.5 A batch's pre-batch formula-ness is read off its first entry, so the journal-maintained formula count agrees with a recount | test | `journal_accounts_a_batch_against_its_pre_batch_state` |
| I2.6 A cell that stops being a formula loses its outgoing edges, through every write API | test | `journal_value_over_formula_drops_edges` |
| I2.7 Overwriting a cell re-dirties the readers of its formula text | test | `formulatext_sees_value_overwrite_of_its_argument`, `isformula_sees_value_overwrite_of_its_argument` (they used to double as I1.5's witnesses; I1.5 is constructional now, this clause is what keeps them) |
| I2.8 A link write and a link removal journal, from each producer, including a link stranded at a position with no cell | test, one per producer | `cell_link_write_is_reported_in_the_delta`, `range_clear_reports_a_stranded_link_removal` |
| I2.9 A hidden-flag write dirties the readers of that line | test | `incremental_subtotal_sees_hidden_row` (it used to double as I1.6's witness; the recording half is constructional now, this clause is what keeps it) |
| I2.10 A rejected write journals nothing | test | `journal_rejected_write_logs_nothing` |
| I2.11 Undo's writes journal, including through a paused evaluation | test | `incremental_undo_under_pause_stays_correct` |

### I3 — a stored value is served only where it is a function of that cell's inputs

| Clause | Enforcement | Witness |
|---|---|---|
| I3.1 Cells on a cycle and everything downstream are seeded dirty every pass | test + oracle | `covfuzz_count_offset_cycle_lost_after_number_overwrite` |
| I3.2 That set is rebuilt over the whole graph after a full pass, so a cycle no cone would seed is still known | test | the same shape (it dies to both mutants) |
| I3.3 A blocked (`#SPILL!`) anchor stays in the array index | test | `a_blocked_anchors_reader_is_recomputed_only_by_a_full_pass` |
| I3.4 The blocked-reader set is rebuilt on every full pass | test | the same |
| I3.5 Verify's stored-vs-live skip list is the never-served list | construction | — |
| I3.6 A blank formula result is stored as the 0 a live re-evaluation produces | test + oracle | `stored_empty_formula_is_live_zero` |

### I4 — the selective pass produces Full's result, pass for pass

| Clause | Enforcement | Witness |
|---|---|---|
| I4.1 The cone is closed under cell, range and input dependents; the banded range index answers the point query exactly and prunes what it drops | test + oracle | `range_index_matches_brute_force`, `removing_last_dependent_prunes_the_index` |
| I4.2 The acyclic cone is ordered by edges, not position | test | `acyclic_cone_orders_a_scalar_anchor_by_edges_not_position` (the anchor sits *below* its reader, so a positional walk gets it wrong) |
| I4.3 The known-cycle cone is walked in Full's own two phases | test | `new_cycle_around_an_anchor_places_circ_like_full_phase_one` |
| I4.4 Each cone cell is evaluated at most once per pass | test | `incremental_does_not_re_evaluate_a_mid_cycle_cell` |
| I4.5 The change key is `(type, number bits, link)`, and the cutoff stops at a cell whose key did not move | test, one per component | `incremental_propagates_error_to_text_transition`, `incremental_reports_signed_zero_flip`, `incremental_reports_dynamic_link_retarget`; the cutoff itself is `incremental_reports_only_value_changes` |
| I4.6 Edges are the reads of *this* pass, replacing the last pass's | test | `incremental_tracks_dynamic_branch_dependencies` |
| I4.7 An out-of-cone read is answered from the store | construction + oracle | — |
| I4.8 A cell that no longer resolves to `HYPERLINK` drops its link | test | `incremental_rebuilds_dynamic_links` (in `test_fn_hyperlink.rs`) |

### I5 — a structural edit rewrites every positional index

The remapping is deliberately conservative: an index that keeps a stale entry costs a redundant recompute or one full fallback, never a wrong value, because a dirtied cell re-records its reads from scratch. That argument covers *which* entries survive; it does not cover *where* they land, so the two comparisons that decide that are pinned directly.

| Clause | Enforcement | Witness |
|---|---|---|
| I5.1 No positional index is forgotten | construction (`DependencyGraph::shift` destructures `Self`) | — a test asserting "index X shifted" is tautological and is not kept |
| I5.2 The reverse index (`precedents`) and the dynamic-link map move with the cells they key | test | `incremental_structural_edit_moves_volatile_with_the_graph`, `incremental_insert_moves_hyperlink_with_the_cell` |
| I5.3 A delete that would only shrink a tracked range rebuilds instead of shifting — and both ends of that overlap test are exact | test | `incremental_row_delete_shrinking_range_forces_full`; the band edges are `shrink_detection_is_exact_at_the_band_edges` |
| I5.4 A structural edit dirties what can change with no precedent moving | **redundant second path** — see below | the journal is the primary; `name_reader_redirty_on_insert` covers the name half by way of its cell edge |
| I5.5 The remapping rule is exact at the edit boundary | test | `displacement_remaps_at_the_edit_boundary` |

**On I5.4.** `mark_structural_dependents` marks four things beyond the cell-edge half: range dependents, `Name`/`SheetStructure`/`Computed` readers, and `OwnCoord`/`FormulaText` readers. Each of the four can be deleted with nothing failing, and so can all four at once — the dirty set after an insert is still non-empty and still correct. The primary path is the journal, in two parts: `displace_cells` rewrites and journals every formula whose *text* the edit changes (which is every literal range that moves, and which re-dirties `FORMULATEXT` readers through the drain's own text-reader marking), and the worksheet's row/column shift journals every non-blank cell that physically moves, so ordinary reachability through the already-shifted edges reaches the dependents of everything that moved — through a rectangle included. A computed reference is the sharpest case and it resolves the same way: `=OFFSET($C$1,5,0)` above an insert keeps its text, but the cells it resolves through are journaled, and its shifted rect still reaches one of them.

The four are kept anyway, as belt over braces, because the subsumption argument is empirical rather than structural: it says the journal happens to cover every shape anyone has constructed, not that it must. The oracle is what carries them — `base/tests/common/` now plants whole-column reads on the data columns, defined names targeting whole columns and rows, bounded counts that notice a blank an insert adds, and `OFFSET`/`INDIRECT`/`ROW`/`FORMULATEXT` forms whose text is displacement-stable, which is exactly the shape class where the journal is silent and only this marking speaks.

Half of that shape class could not be planted at first: `COUNTA`, `COUNTBLANK`, `COUNT`, `SUBTOTAL`, `AVERAGE`, `MAX` and `MIN` walked all 1,048,576 rows of a whole-column reference instead of clipping it to the used range the way `SUM`, `COUNTIF` and `SUMIF` did, which cost about 250ms an evaluate against 0.25ms and made the generator 126x slower. Every aggregate range walk goes through `EvalCtx::clip_range_to_used` now — including the whole-row form, whose *column* axis was never clipped by anything — and the dropped templates and whole-column name targets are back. The cost clause is `whole_column_aggregate_cost_tracks_the_used_range` in `recalc_cost.rs`; the one function whose answer depends on the clipped-away cells is `COUNTBLANK`, whose remainder arithmetic is `countblank_adds_back_the_cells_the_clip_removed`.

The extension paid for itself before it ever guarded anything: it turned the 40×120 run red on seed 3 and minimized to eight operations around `=OFFSET($C$1,3,0)`. The engine was right and the harness was wrong — `Op::value_seed` did not count a quote-prefixed write as a user edit, so a legitimate delta entry looked unsound. Which is the point of planting a shape class: the first thing it finds need not be the thing it was aimed at.

### I6 — the delta is sound and complete

| Clause | Enforcement | Witness |
|---|---|---|
| I6.1 Delta ⊇ moved; Delta ⊆ moved ∪ seeds ∪ volatile cone; a user edit is reported even when its own value did not move; dirty is not report | oracle + test | `subtotal_formula_text_reread_is_not_always_reported` |
| I6.2 Every always-dirty cell is reported on every pass, asserted against the **pre-pass** set | oracle; the pre-vs-post choice is test | `verify_liveness_allows_a_cell_that_becomes_volatile_mid_pass`, `verify_liveness_still_binds_when_a_cell_leaves_volatility` |
| I6.3 `Everything` where a cell list cannot name what moved, and the flag dies with its pass | test | `take_changed_cells_reports_everything_for_data_only_shift`, and `take_changed_cells_reports_everything_for_trailing_delete` for the Ready-with-empty-dirty branch |
| I6.4 A redundant full pass keeps the delta it inherited unless values moved; a volatile makes them move | test | `take_changed_cells_survives_redundant_evaluate`, `redundant_evaluate_keeps_rand_reporting_but_not_sumifs` |
| I6.7 A debt-forced full pass carrying a pending edit answers `Everything`, not a delta the edit is missing from | oracle | the fuzzer catches it on seed 1 in nine operations |
| I6.5 A conditional-format result that moves enters the delta, whether a value drove it or a rule edit did | test, one per driver | `incremental_reports_conditional_format_change`, `incremental_reports_cf_only_mutation` |
| I6.6 Reading the delta re-arms it | test | `take_changed_cells_reports_incremental_delta` |

### I7 — default Full mode is what it was before the engine existed

| Clause | Enforcement | Witness |
|---|---|---|
| I7.1 Tracing and the graph are off in Full | construction | — |
| I7.2 The pre-existing suite runs in Full by default | oracle (the whole `--lib` run) | — |
| I7.3 A structural rebuild path suspends the CSE member guard | gate for the obligation, construction for the bypass, test per axis | `unchecked_rebuild_paths_suspend_the_cse_member_guard`; `moving_a_column_with_a_cse_anchor_always_succeeds`, `moving_a_row_with_a_cse_anchor_always_succeeds` |
| I7.4 `range_clear_all` tears a footprint down through the style-preserving primitive | gate + clippy ban + test | `range_clear_all_spill_teardown_preserves_style`; `clear_all_over_part_of_a_spill_drops_only_the_selected_styles` in `test_clear_cells.rs` |
| I7.5 A write into a CSE member position is rejected, in both modes | test | `writes_into_cse_members_are_rejected` in `test_arrays_formulas.rs` |

### I8 — every fallback fires when its condition holds

| Clause | Enforcement | Witness |
|---|---|---|
| I8.1 The graph is not ready: a reparse (names, sheets), a locale or timezone change | test, one per call site | `incremental_defined_name_retarget_forces_full`, `incremental_set_locale_forces_full` |
| I8.2 The previous pass left convergence debt | test | `incremental_heals_spill_debt_left_by_a_forced_full_pass` (the condition has one arm now; the cycle-through-the-array arm is gone — see Convergence debt) |
| I8.3 The cone reaches more than half the formulas, and the fallback actually runs a pass | test | `incremental_wide_fanout_stays_correct` |
| I8.4 The cone reaches a reader of a blocked spill anchor | test | `a_blocked_anchors_reader_is_recomputed_only_by_a_full_pass` |
| I8.5 The cone reaches an array footprint; and a 1x1 dynamic anchor is *not* one | test, one per direction | `incremental_overwrite_spill_anchor_updates_dependents`, `scalar_result_dynamic_anchors_stay_incremental` |
| I8.6 The pass reported `#CIRC!` for a cycle the graph did not contain | test | `incremental_does_not_re_evaluate_a_mid_cycle_cell` |
| I8.7 An evaluation write changed an array footprint | test | `new_cycle_around_an_anchor_places_circ_like_full_phase_one` |
| I8.8 A row, column or cell move forces the next pass full | oracle | the differential fuzzer catches it in four operations on seed 6; no deterministic test does |
| I8.9 A state machine that cannot be half-ready: `mark_dirty` on a `MustRebuild` graph is ignored | test | `graph_state_is_explicit` |

### Gaps this map does not close

Empty. The eighteen mechanisms this section used to list were dispositioned: **eight now die to a witness, two were deleted, and eight are kept as a deliberate second path with the reason recorded** — either here or in the section that owns them. Nothing is left "unwitnessed, no reason given".

Witnessed. `Displacement`'s arithmetic turned out to be five one-sided comparisons, not four, and it is pinned at the unit level (I5.5, I5.3) rather than through a model whose fallbacks mask it: `shift_coord` is the single definition every stored coordinate moves through, and `range_overlaps_band` is the whole of the shrink test. The journal drain's first-entry rule is I2.5, checked against a from-scratch recount, one direction at a time because the two errors cancel. And the `trace_rect` calls turned out **not** to be the benign optimisation this section used to claim: at a wide extent the reader's per-cell walk is clipped to the used range and the rectangle is the only edge, which is I1.3's own lesson reached through a computed extent (I1.8).

Deleted. The convergence-debt condition's `circular` arm, which could not decide anything — a cycle through a footprint puts the anchor in `never_served`, which forces the next pass full through the arrays fallback anyway (see Convergence debt). And `INDIRECT`'s 1×1 rectangle, which is the same edge as the cell read its reader already records, and which as a *range* made a delete of that row rebuild the graph for nothing.

Kept as a second path, reason recorded. `mark_structural_dependents`' four extra halves and the `Name`/`SheetStructure`/`Computed` inputs that feed them: the journal is the primary and covers every constructed shape, but that argument is empirical rather than structural, so the marking stays and the generator now plants the shape class where the journal is silent (see I5.4). `recompute_frontier`'s memo restore bounds work rather than fixing a value — a skipped helper recomputed unscoped returns the same value, by the third design rule — and its second `reports_change` sweep is the delta-completeness net that Verify's own delta check is the oracle for. The arrays fallback's fresh-anchor half is redundant with the post-pass `wrote_array_cells` redo for any anchor that spills; it stays so a fresh anchor gets Full's two-phase ordering on the pass it is first seen, rather than after a wasted incremental attempt. `get_range`'s `trace_rect` is I1.8's rule at the range-composition site.

A caution learned closing these, worth more than any single item: **fuzz silence is not evidence a mechanism is dead.** Deleting all ten of the then-open mechanisms at once left the lib suite green in all three modes and the differential fuzzer green too — including the two `trace_rect` calls that a twelve-line test proves are load-bearing. A mechanism may be deleted for a *structural* subsumption argument, never for the oracle failing to notice.

Two clauses that no deterministic test covers are squarely in the **oracle** column rather than here: the row/column-move fallback (I8.8) dies on seed 6 in four operations, and the `debt_over_pending_edits` branch of I6 dies on seed 1 in nine.

Closing a gap means adding the minimal witness, not a shape; a shape that dies to no mutant is not a witness.

## Test discipline

Every test in this engine's suite must be the minimal witness of one clause above: the name says what
breaks, a doc comment says why it matters when that is not obvious, and the shape is the smallest one that
fails when the mechanism is wrong. Before adding a test, find its clause in the map; if that clause already
has a witness, the test is redundant however differently it reads. Before deleting or merging tests,
re-apply the relevant mutants (see the nightly-recalc-audit workflow) and confirm nothing that used to die
now survives. Redundancy is measured by kill-power, not by reading similarity — and "it exercises a
different formula" is not kill-power, because the fuzz generator already varies the formula.

That rule is what the map is for, and it is load-bearing: an audit that derived the clause list from the
mechanisms and scored every test against a 66-mutant catalog found that 61 of the 111 tests in
`test_incremental_recalc.rs` had a kill set already covered by another, pinned a scenario value with no
mechanism behind it, or asserted something the compiler now settles. They are in the history; the map is
what replaces them.

Where a test goes follows from that. The default is the lib suite: it is what
`IRONCALC_RECALC=incremental`/`verify` re-run under a different strategy and what the nightly
`cargo mutants` job executes, so a test outside it is a test those two oracles never see. Each
`base/tests/*.rs` file is also its own compile-and-link, so a new one costs build time on every
`cargo test`. Only three things earn a place there:

- `fuzz_differential.rs` and `common/`, the lockstep harness and generator.
- `fuzz_covfuzz_regressions.rs`, minimized fuzz artifacts replayed through that harness — one shape
  per kill class, each doc comment naming the mutant it dies to.
- `recalc_cost.rs`, the one wall-clock invariant, kept out of the lib suite so the mutation job does
  not pay for a 32k-cell workbook once per mutant.
