#![allow(clippy::unwrap_used)]

// Property test: incremental recalculation gives what full recalculation
// gives.
//
// Two `UserModel`s, one recalculating incrementally and one in full, go
// through the same random sequence of actions: constants, formulas of every
// kind that reads cells differently, array formulas, clearing, inserting,
// deleting and moving rows and columns, hiding rows, renaming and adding
// sheets, undo and redo. A `UserModel` evaluates after every action, like the
// UI. After each one the two must show the same thing in every cell.
//
// `IRONCALC_FUZZ_SEEDS=5000 cargo test recalc_fuzz` runs more of them.

use crate::expressions::types::Area;
use crate::test::util::new_empty_model;
use crate::{RecalcMode, UserModel};

const ROWS: i32 = 8;
const COLUMNS: i32 = 6;
const STEPS: usize = 40;
const SEEDS: u64 = 300;

/// With the `recalc_verify` feature every incremental evaluation is also
/// checked against a full one from inside, graph included.
#[cfg(not(feature = "recalc_verify"))]
const INCREMENTAL: RecalcMode = RecalcMode::Incremental;
#[cfg(feature = "recalc_verify")]
const INCREMENTAL: RecalcMode = RecalcMode::Verify;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn between(&mut self, low: i32, high: i32) -> i32 {
        low + self.below((high - low + 1) as u64) as i32
    }
    fn cell(&mut self) -> String {
        let column = (b'A' + self.below(COLUMNS as u64) as u8) as char;
        format!("{column}{}", self.between(1, ROWS))
    }
    fn column(&mut self) -> char {
        (b'A' + self.below(COLUMNS as u64) as u8) as char
    }
}

fn formula(rng: &mut Rng) -> String {
    let (a, b, c) = (rng.cell(), rng.cell(), rng.cell());
    let column = rng.column();
    let n = rng.between(1, 4);
    match rng.below(34) {
        0 => format!("={a}+{b}"),
        1 => format!("=SUM({a}:{b})"),
        2 => format!("=SUM({column}:{column})"),
        3 => format!("=SUM({n}:{n})"),
        4 => format!("=SEQUENCE({n})"),
        5 => format!("=SEQUENCE({a})"),
        6 => format!("=SEQUENCE({n},2)"),
        7 => format!("={a}:{b}"),
        8 => format!("=OFFSET({a},{},{})", rng.between(-2, 2), rng.between(-2, 2)),
        9 => format!("=INDIRECT(\"{a}\")"),
        10 => format!("=INDEX({a}:{b},{n},1)"),
        11 => "=ROW()".to_string(),
        12 => format!("=COLUMN({a})"),
        13 => format!("=IF({a}>2,{b},{c})"),
        14 => format!("=IFERROR({a}/{b},0)"),
        15 => format!("=COUNTBLANK({a}:{b})"),
        16 => format!("=SUBTOTAL(109,{a}:{b})"),
        17 => format!("=Sheet2!{a}+1"),
        18 => format!("=SUMIF({column}:{column},\">1\")"),
        19 => format!("=TRANSPOSE({a}:{b})"),
        20 => format!("=ISFORMULA({a})"),
        21 => format!("=FORMULATEXT({a})"),
        22 => format!("=LET(x,{a},x*2)"),
        23 => format!("={a}&{b}"),
        24 => format!("=MAX({a}:{b},{c})"),
        25 => format!("=XLOOKUP({n},{column}1:{column}{ROWS},{a}:{b})"),
        26 => format!("=SORT({a}:{b})"),
        27 => format!("=UNIQUE({column}1:{column}{ROWS})"),
        28 => format!("=COUNTIF({column}:{column},{n})"),
        29 => format!("=ROWS({a}:{b})+COLUMNS({a}:{b})"),
        30 => format!("=SUM(OFFSET({a},0,0,{n},1))"),
        31 => format!("=IF(ISBLANK({a}),\"free\",\"taken\")"),
        32 => format!("=HYPERLINK(\"https://example.com/\"&{a},{b})"),
        _ => format!("=MATCH({n},{column}1:{column}{ROWS},0)"),
    }
}

/// Applies one random action to a model and says what it was and whether the
/// model took it.
fn act(model: &mut UserModel, rng: &mut Rng) -> (String, bool) {
    let sheet_count = model.model.workbook.worksheets.len() as u64;
    let sheet = rng.below(sheet_count.min(2)) as u32;
    let row = rng.between(1, ROWS);
    let column = rng.between(1, COLUMNS);
    let count = rng.between(1, 2);
    let (what, result) = match rng.below(100) {
        0..=24 => {
            let value = match rng.below(5) {
                0 => "x".to_string(),
                1 => "TRUE".to_string(),
                _ => rng.between(0, 5).to_string(),
            };
            (
                format!("set {sheet} {row} {column} {value}"),
                model.set_user_input(sheet, row, column, &value),
            )
        }
        25..=54 => {
            let formula = formula(rng);
            (
                format!("set {sheet} {row} {column} {formula}"),
                model.set_user_input(sheet, row, column, &formula),
            )
        }
        55..=61 => {
            let area = Area {
                sheet,
                row,
                column,
                width: count,
                height: count,
            };
            (
                format!("clear {sheet} {row} {column} {count}x{count}"),
                model.range_clear_contents(&area),
            )
        }
        62..=64 => {
            let formula = formula(rng);
            (
                format!("cse {sheet} {row} {column} {count}x2 {formula}"),
                model.set_user_array_formula(sheet, row, column, count, 2, &formula),
            )
        }
        65..=69 => (
            format!("insert rows {sheet} {row} {count}"),
            model.insert_rows(sheet, row, count),
        ),
        70..=74 => (
            format!("delete rows {sheet} {row} {count}"),
            model.delete_rows(sheet, row, count),
        ),
        75..=78 => (
            format!("insert columns {sheet} {column} {count}"),
            model.insert_columns(sheet, column, count),
        ),
        79..=82 => (
            format!("delete columns {sheet} {column} {count}"),
            model.delete_columns(sheet, column, count),
        ),
        83..=85 => {
            let delta = rng.between(-2, 2);
            (
                format!("move rows {sheet} {row} {count} by {delta}"),
                model.move_rows_action(sheet, row, count, delta),
            )
        }
        86..=87 => {
            let delta = rng.between(-2, 2);
            (
                format!("move columns {sheet} {column} {count} by {delta}"),
                model.move_columns_action(sheet, column, count, delta),
            )
        }
        88..=90 => {
            let hidden = rng.below(2) == 0;
            (
                format!("hide rows {sheet} {row} {hidden}"),
                model.set_rows_hidden(sheet, row, row, hidden),
            )
        }
        91..=95 => ("undo".to_string(), model.undo()),
        96..=97 => ("redo".to_string(), model.redo()),
        98 => ("new sheet".to_string(), model.new_sheet()),
        _ => {
            let name = format!("Renamed{}", rng.below(3));
            (
                format!("rename sheet 0 {name}"),
                model.rename_sheet(0, &name),
            )
        }
    };
    (what, result.is_ok())
}

/// What a model shows: for every cell that holds something or lies in the
/// part of the sheets the actions reach, what it holds and what it displays.
fn snapshot(model: &UserModel) -> Vec<String> {
    let mut lines = Vec::new();
    for sheet in 0..model.model.workbook.worksheets.len() as u32 {
        for row in 1..=ROWS + 8 {
            for column in 1..=COLUMNS + 6 {
                let content = model.get_cell_content(sheet, row, column).unwrap();
                let value = model.get_formatted_cell_value(sheet, row, column).unwrap();
                if !content.is_empty() || !value.is_empty() {
                    lines.push(format!("{sheet} {row} {column}: {content} -> {value}"));
                }
            }
        }
    }
    lines
}

#[test]
fn incremental_recalculation_gives_what_full_recalculation_gives() {
    let seeds = std::env::var("IRONCALC_FUZZ_SEEDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(SEEDS);
    let mut failures = Vec::new();
    for seed in 1..=seeds {
        let mut full = UserModel::from_model(new_empty_model().with_recalc_mode(RecalcMode::Full));
        let mut incremental =
            UserModel::from_model(new_empty_model().with_recalc_mode(INCREMENTAL));
        full.new_sheet().unwrap();
        incremental.new_sheet().unwrap();
        let mut history = Vec::new();
        for step in 0..STEPS as u64 {
            let state = (seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
                ^ step.wrapping_mul(0xD6E8_FEB8_6659_FD93))
                | 1;
            let (what, taken) = act(&mut full, &mut Rng(state));
            let (_, taken_incrementally) = act(&mut incremental, &mut Rng(state));
            history.push(what);
            // Not every action evaluates: hiding a row does not.
            full.evaluate();
            incremental.evaluate();
            let (expected, found) = (snapshot(&full), snapshot(&incremental));
            // Whatever it shows, what the incremental model holds must be what
            // its formulas give, wherever that is so of the full model.
            let known = crate::test::dynamic_evaluation::oracle::violations(&mut full.model);
            let mut violations =
                crate::test::dynamic_evaluation::oracle::violations(&mut incremental.model);
            violations.retain(|violation| !known.contains(violation));
            // Which of two arrays that want the same cells got there first,
            // and whether a loop through the area of an array is reported
            // circular or blocked, depends on the order arrays were evaluated
            // in before, in a full evaluation as well. States that show either
            // are allowed to differ.
            let history_dependent = expected
                .iter()
                .chain(found.iter())
                .any(|line| line.ends_with("#SPILL!") || line.ends_with("#CIRC!"));
            let same = taken == taken_incrementally && (history_dependent || expected == found);
            if !same || !violations.is_empty() {
                let differences: Vec<String> = expected
                    .iter()
                    .filter(|line| !found.contains(line))
                    .map(|line| format!("full:        {line}"))
                    .chain(
                        found
                            .iter()
                            .filter(|line| !expected.contains(line))
                            .map(|line| format!("incremental: {line}")),
                    )
                    .chain(
                        violations
                            .iter()
                            .map(|line| format!("inconsistent: {line}")),
                    )
                    .collect();
                failures.push(format!(
                    "seed {seed}, after {} actions:\n  {}\n{}",
                    history.len(),
                    history.join("\n  "),
                    differences.join("\n")
                ));
                break;
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {seeds} histories differ. The first:\n\n{}",
        failures.len(),
        failures[0]
    );
}
