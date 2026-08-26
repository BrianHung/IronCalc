#!/bin/zsh
# Differential clippy run with codex-rs's workspace deny-list (as warnings).
source "$HOME/.cargo/env" 2>/dev/null
TREE=$1; OUT=$2
cd $TREE
LINTS=(await_holding_invalid_type await_holding_lock expect_used identity_op manual_clamp manual_filter manual_find manual_flatten manual_map manual_memcpy manual_non_exhaustive manual_ok_or manual_range_contains manual_retain manual_strip manual_try_fold manual_unwrap_or needless_borrow needless_borrowed_reference needless_collect needless_late_init needless_option_as_deref needless_question_mark needless_update redundant_clone redundant_closure redundant_closure_for_method_calls redundant_static_lifetimes trivially_copy_pass_by_ref uninlined_format_args unnecessary_filter_map unnecessary_lazy_evaluations unnecessary_sort_by unnecessary_to_owned unwrap_used)
FLAGS=()
for l in $LINTS; do FLAGS+=("-W" "clippy::$l"); done
cargo clippy -p ironcalc_base --lib --message-format=json -- $FLAGS 2>/dev/null \
 | jq -r 'select(.reason=="compiler-message") | .message | select(.code != null) | select(.code.code | startswith("clippy::")) | .code.code + " " + (.spans[] | select(.is_primary) | .file_name + ":" + (.line_start|tostring))' \
 | sort -u > $OUT
wc -l < $OUT
