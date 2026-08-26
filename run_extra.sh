#!/bin/zsh
source "$HOME/.cargo/env" 2>/dev/null
LOG=/private/tmp/fix90/extra_results.log
: > $LOG
cd /private/tmp/fix90
echo "===== X86_64 FULL lib =====" >> $LOG
cargo test -p ironcalc_base --lib --target x86_64-apple-darwin >> $LOG 2>&1
echo "EXIT: $?" >> $LOG
echo "===== WASM tests =====" >> $LOG
cd bindings/wasm && make tests >> $LOG 2>&1
echo "EXIT: $?" >> $LOG
echo "ALL DONE" >> $LOG
