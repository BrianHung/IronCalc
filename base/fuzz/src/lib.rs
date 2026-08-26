//! Structure-aware byte decoder for the coverage-guided differential fuzzer.
//!
//! Bytes 0..2 select a `Generator::setup` seed (a rich, deterministic workbook:
//! data region, formula zoo, defined names, spill anchors at E15/F15, the
//! G19/H19 circular pair). The remaining bytes decode into a bounded tail of
//! ops, byte-per-decision, so libFuzzer mutation moves the sequence through
//! op-kind, coordinate and formula space directly. Op kinds oversample the
//! historically buggy product space: cycles x spill anchors x structural edits
//! x error-absorbing formulas x multi-pass evaluation.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../../tests/common/mod.rs"]
pub mod common;

use common::{col_letter, ops_to_rust, Generator, GenConfig, Op, ZOO, DATA_ZOO, NAME_POOL, NAME_TARGETS};
use std::collections::HashMap;
use std::sync::Mutex;

/// Maximum decoded tail ops (excluding the setup prefix and interleaved
/// Evaluates).
pub const MAX_TAIL_OPS: usize = 48;

/// Formulas oversampled on top of the zoo: known-cycle members, spill-anchor
/// consumers, error absorbers, moving-reference reads.
pub const TARGETED: &[&str] = &[
    "=G19+1",
    "=E20",
    "=SUM(E19:E20)",
    "=H19+1",
    "=E15#",
    "=SUM(E15#)",
    "=COUNT(F15#)",
    "=SUM(F15#)+1",
    "=IFERROR({F},-1)",
    "=ISERROR({F})",
    "=IF(ISERROR({F}),1,0)",
    "=IFERROR(E15#,9)",
    "=IFERROR(G19,7)",
    "=INDIRECT(\"E15\")",
    "=SUM(INDIRECT(\"E15:E17\"))",
    "=OFFSET(E15,1,0)",
    "=SUM(OFFSET(E15,0,0,3,1))",
    "=SEQUENCE(MOD(A1,3)+1)",
    "=SUM(E19:E20)+SUM(E15#)",
    "=FORMULATEXT(E15)",
    "=ISFORMULA(F15)",
    "=SUBTOTAL(9,E15:E17)",
];

struct Bytes<'a> {
    d: &'a [u8],
    i: usize,
}

impl<'a> Bytes<'a> {
    fn new(d: &'a [u8]) -> Self {
        Bytes { d, i: 0 }
    }
    fn u8(&mut self) -> u8 {
        let v = self.d.get(self.i).copied().unwrap_or(0);
        self.i += 1;
        v
    }
    fn done(&self) -> bool {
        self.i >= self.d.len()
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.u8() as i32) % (hi - lo + 1)
    }
    fn pick<'b, T>(&mut self, items: &'b [T]) -> &'b T {
        &items[self.u8() as usize % items.len()]
    }
}

/// Deterministic placeholder fill driven by input bytes (the shared
/// `Generator::fill` draws from its own RNG, which would decouple bytes from
/// behavior and blind the coverage feedback).
fn fill(template: &str, b: &mut Bytes, data_sheet: &str) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let close = rest[i..].find('}').map(|j| i + j).unwrap_or(rest.len() - 1);
        let key = &rest[i + 1..close];
        let rep = match key {
            "D" => data_sheet.to_string(),
            "R" => b.range(1, common::DATA_ROWS).to_string(),
            "S" => b.range(1, 6).to_string(),
            "C" => col_letter(b.range(1, common::DATA_COLS)),
            "F" => format!(
                "{}{}",
                col_letter(b.range(common::FORMULA_COL_FIRST, common::FORMULA_COL_LAST)),
                b.range(1, common::FORMULA_ROWS)
            ),
            "N" => b.range(0, 9).to_string(),
            "M" => b.range(1, 5).to_string(),
            other => format!("{{{other}}}"),
        };
        out.push_str(&rep);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

fn value_from(b: &mut Bytes) -> String {
    match b.u8() % 12 {
        0 => String::new(),
        1 => "TRUE".into(),
        2 => "FALSE".into(),
        3 => (*b.pick(&["x", "abc", "hello", "10", "a1"])).to_string(),
        4 => "'7".into(),
        5 => "-5".into(),
        6 => "1000".into(),
        7 => "0".into(),
        8 => "#N/A".into(),
        9 => "https://ironcalc.com".into(),
        _ => b.range(0, 99).to_string(),
    }
}

fn setup_ops(seed: u16) -> Vec<Op> {
    static CACHE: Mutex<Option<HashMap<u16, Vec<Op>>>> = Mutex::new(None);
    let mut guard = CACHE.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    map.entry(seed)
        .or_insert_with(|| {
            let mut g = Generator::new(
                seed as u64,
                GenConfig {
                    steps: 0,
                    ..GenConfig::default()
                },
            );
            g.setup();
            g.ops
        })
        .clone()
}

fn sheet_from(b: &mut Bytes) -> u32 {
    match b.u8() % 6 {
        0 => 1,
        1 => 2,
        _ => 0,
    }
}

/// Decodes an input into a full scenario (setup prefix + op tail). Returns
/// `None` when the input is too short to mean anything.
pub fn decode(data: &[u8]) -> Option<Vec<Op>> {
    if data.len() < 4 {
        return None;
    }
    let mut b = Bytes::new(data);
    let seed = u16::from_le_bytes([b.u8(), b.u8()]);
    let mut ops = setup_ops(seed);
    let mut data_name = "Data".to_string();
    let mut tail = 0usize;
    while !b.done() && tail < MAX_TAIL_OPS {
        tail += 1;
        let op = match b.u8() % 24 {
            // -- formula writes on Sheet1 (zoo, byte-indexed)
            0..=4 => {
                let t = *b.pick(ZOO);
                Op::Set {
                    sheet: 0,
                    row: b.range(1, 16),
                    col: b.range(1, 8),
                    value: fill(t, &mut b, &data_name),
                }
            }
            // -- targeted pool: cycles x anchors x absorbers (oversampled)
            5..=7 => {
                let t = *b.pick(TARGETED);
                Op::Set {
                    sheet: 0,
                    row: b.range(1, 16),
                    col: b.range(1, 8),
                    value: fill(t, &mut b, &data_name),
                }
            }
            8 => Op::Set {
                sheet: sheet_from(&mut b),
                row: b.range(1, 16),
                col: b.range(1, 8),
                value: value_from(&mut b),
            },
            9 => match b.u8() % 3 {
                0 => Op::SetNumber {
                    sheet: sheet_from(&mut b),
                    row: b.range(1, 16),
                    col: b.range(1, 8),
                    value: b.range(-5, 120) as f64,
                },
                1 => Op::SetText {
                    sheet: sheet_from(&mut b),
                    row: b.range(1, 16),
                    col: b.range(1, 8),
                    value: "t".into(),
                },
                _ => Op::SetBool {
                    sheet: sheet_from(&mut b),
                    row: b.range(1, 16),
                    col: b.range(1, 8),
                    value: b.u8() % 2 == 0,
                },
            },
            10 => Op::ArrayFormula {
                sheet: 0,
                row: b.range(1, 16),
                col: b.range(1, 8),
                width: b.range(1, 2),
                height: b.range(1, 3),
                formula: (*b.pick(&["=A1:A2*2", "=A1:A3+1", "=SUM(A1:A3)", "=B1:B2"])).to_string(),
            },
            // -- structural edits
            11 => Op::InsertRows {
                sheet: sheet_from(&mut b),
                row: b.range(1, 22),
                count: b.range(1, 2),
            },
            12 => Op::DeleteRows {
                sheet: sheet_from(&mut b),
                row: b.range(1, 22),
                count: b.range(1, 2),
            },
            13 => Op::InsertCols {
                sheet: sheet_from(&mut b),
                col: b.range(1, 8),
                count: 1,
            },
            14 => Op::DeleteCols {
                sheet: sheet_from(&mut b),
                col: b.range(1, 8),
                count: 1,
            },
            15 => Op::MoveRows {
                sheet: sheet_from(&mut b),
                row: b.range(1, 20),
                count: b.range(1, 2),
                delta: *b.pick(&[-3, -1, 1, 2, 4]),
            },
            16 => Op::MoveCols {
                sheet: sheet_from(&mut b),
                col: b.range(1, 7),
                count: 1,
                delta: *b.pick(&[-2, -1, 1, 2]),
            },
            // -- defined names
            17 => {
                let (name, scope) = *b.pick(NAME_POOL);
                let target = *b.pick(NAME_TARGETS);
                let formula = fill(target, &mut b, &data_name);
                match b.u8() % 3 {
                    0 => Op::NewName {
                        name: name.into(),
                        scope,
                        formula,
                    },
                    1 => {
                        let (new_name, new_scope) = *b.pick(NAME_POOL);
                        Op::UpdateName {
                            name: name.into(),
                            scope,
                            new_name: new_name.into(),
                            new_scope,
                            formula,
                        }
                    }
                    _ => Op::DeleteName {
                        name: name.into(),
                        scope,
                    },
                }
            }
            // -- sheet ops
            18 => match b.u8() % 4 {
                0 => Op::AddSheet { name: "Tmp".into() },
                1 => Op::DeleteSheet {
                    index: b.u8() as u32 % 4,
                },
                _ => {
                    let name = if data_name == "Data" { "Data2" } else { "Data" };
                    data_name = name.to_string();
                    Op::RenameSheet {
                        index: 1,
                        name: name.into(),
                    }
                }
            },
            19 => {
                let area = (
                    sheet_from(&mut b),
                    b.range(1, 20),
                    b.range(1, 8),
                    b.range(1, 3),
                    b.range(1, 3),
                );
                if b.u8() % 2 == 0 {
                    Op::ClearContents { area }
                } else {
                    Op::ClearAll { area }
                }
            }
            20 => {
                if b.u8() % 2 == 0 {
                    Op::HideRow {
                        sheet: sheet_from(&mut b),
                        row: b.range(1, 14),
                        hidden: b.u8() % 2 == 0,
                    }
                } else {
                    Op::HideCol {
                        sheet: sheet_from(&mut b),
                        col: b.range(1, 8),
                        hidden: b.u8() % 2 == 0,
                    }
                }
            }
            21 => Op::CellStyle {
                sheet: sheet_from(&mut b),
                row: b.range(1, 16),
                col: b.range(1, 8),
                variant: b.u8() % 4,
            },
            // -- Data-sheet formula writes
            22 => {
                let t = *b.pick(DATA_ZOO);
                Op::Set {
                    sheet: 1,
                    row: b.range(1, 8),
                    col: b.range(1, 5),
                    value: fill(t, &mut b, &data_name),
                }
            }
            _ => Op::Evaluate,
        };
        ops.push(op);
        if b.u8() % 3 == 0 {
            ops.push(Op::Evaluate);
        }
    }
    ops.push(Op::Evaluate);
    Some(ops)
}

/// Shared entry: decode and run the lockstep comparison, panicking on any
/// divergence with a pasteable Op-list repro.
pub fn run_bytes(data: &[u8]) {
    let Some(ops) = decode(data) else { return };
    if let Err(f) = common::run_scenario(&ops, true, cfg!(feature = "recalc_verify")) {
        panic!("DIVERGENCE {f}\nops:\n{}", ops_to_rust(&ops));
    }
}
