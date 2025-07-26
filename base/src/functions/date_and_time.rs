use chrono::DateTime;
use chrono::Datelike;
use chrono::Months;
use chrono::Timelike;

use crate::constants::MAXIMUM_DATE_SERIAL_NUMBER;
use crate::constants::MINIMUM_DATE_SERIAL_NUMBER;
use crate::expressions::types::CellReferenceIndex;
use crate::formatter::dates::date_to_serial_number;
use crate::formatter::dates::permissive_date_to_serial_number;
use crate::model::get_milliseconds_since_epoch;
use crate::{
    calc_result::CalcResult, constants::EXCEL_DATE_BASE, expressions::parser::Node,
    expressions::token::Error, formatter::dates::from_excel_date, model::Model,
};

fn parse_day_simple(day_str: &str) -> Result<u32, String> {
    let bytes_len = day_str.len();
    if bytes_len == 0 || bytes_len > 2 {
        return Err("Not a valid day".to_string());
    }
    match day_str.parse::<u32>() {
        Ok(y) => Ok(y),
        Err(_) => Err("Not a valid day".to_string()),
    }
}

fn parse_month_simple(month_str: &str) -> Result<u32, String> {
    let bytes_len = month_str.len();
    if bytes_len == 0 {
        return Err("Not a valid month".to_string());
    }
    if bytes_len <= 2 {
        // Numeric month representation. Ensure it is within the valid range 1-12.
        return match month_str.parse::<u32>() {
            Ok(m) if (1..=12).contains(&m) => Ok(m),
            _ => Err("Not a valid month".to_string()),
        };
    }

    // Textual month representations.
    // Use standard 3-letter abbreviations (e.g. "Sep") but also accept the legacy "Sept".
    let month_names_short = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month_names_long = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    if let Some(m) = month_names_short
        .iter()
        .position(|&r| r.eq_ignore_ascii_case(month_str))
    {
        return Ok(m as u32 + 1);
    }
    // Special-case the non-standard abbreviation "Sept" so older inputs still work.
    if month_str.eq_ignore_ascii_case("Sept") {
        return Ok(9);
    }

    if let Some(m) = month_names_long
        .iter()
        .position(|&r| r.eq_ignore_ascii_case(month_str))
    {
        return Ok(m as u32 + 1);
    }
    Err("Not a valid month".to_string())
}

fn parse_year_simple(year_str: &str) -> Result<i32, String> {
    let bytes_len = year_str.len();
    if bytes_len != 2 && bytes_len != 4 {
        return Err("Not a valid year".to_string());
    }
    let y = year_str
        .parse::<i32>()
        .map_err(|_| "Not a valid year".to_string())?;
    if y < 30 && bytes_len == 2 {
        Ok(2000 + y)
    } else if y < 100 && bytes_len == 2 {
        Ok(1900 + y)
    } else {
        Ok(y)
    }
}

fn parse_datevalue_text(value: &str) -> Result<i32, String> {
    let separator = if value.contains('/') {
        '/'
    } else if value.contains('-') {
        '-'
    } else {
        return Err("Not a valid date".to_string());
    };

    let mut parts: Vec<&str> = value.split(separator).map(|s| s.trim()).collect();
    if parts.len() != 3 {
        return Err("Not a valid date".to_string());
    }

    // Identify the year: prefer the one that is four-digit numeric, otherwise assume the third part.
    let mut year_idx: usize = 2;
    for (idx, p) in parts.iter().enumerate() {
        if p.len() == 4 && p.chars().all(char::is_numeric) {
            year_idx = idx;
            break;
        }
    }

    let year_str = parts[year_idx];
    // Remove the year from the remaining vector to process day / month.
    parts.remove(year_idx);
    let part1 = parts[0];
    let part2 = parts[1];

    // Helper closures
    let is_numeric = |s: &str| s.chars().all(char::is_numeric);

    // Determine month and day.
    let (month_str, day_str) = if !is_numeric(part1) {
        // textual month in first
        (part1, part2)
    } else if !is_numeric(part2) {
        // textual month in second
        (part2, part1)
    } else {
        // Both numeric – apply disambiguation rules
        let v1: u32 = part1.parse().unwrap_or(0);
        let v2: u32 = part2.parse().unwrap_or(0);
        match (v1 > 12, v2 > 12) {
            (true, false) => (part2, part1), // first cannot be month
            (false, true) => (part1, part2), // second cannot be month
            _ => (part1, part2),             // ambiguous -> assume MM/DD
        }
    };

    let day = parse_day_simple(day_str)?;
    let month = parse_month_simple(month_str)?;
    let year = parse_year_simple(year_str)?;

    match date_to_serial_number(day, month, year) {
        Ok(n) => Ok(n),
        Err(_) => Err("Not a valid date".to_string()),
    }
}

impl Model {
    fn get_date_serial(
        &mut self,
        node: &Node,
        cell: CellReferenceIndex,
    ) -> Result<i64, CalcResult> {
        let result = self.evaluate_node_in_context(node, cell);
        match result {
            CalcResult::Number(f) => Ok(f.floor() as i64),
            CalcResult::String(s) => match parse_datevalue_text(&s) {
                Ok(n) => Ok(n as i64),
                Err(_) => Err(CalcResult::Error {
                    error: Error::VALUE,
                    origin: cell,
                    message: "Invalid date".to_string(),
                }),
            },
            CalcResult::Boolean(b) => {
                if b {
                    Ok(1)
                } else {
                    Ok(0)
                }
            }
            error @ CalcResult::Error { .. } => Err(error),
            CalcResult::Range { .. } => Err(CalcResult::Error {
                error: Error::NIMPL,
                origin: cell,
                message: "Arrays not supported yet".to_string(),
            }),
            CalcResult::EmptyCell | CalcResult::EmptyArg => Ok(0),
            CalcResult::Array(_) => Err(CalcResult::Error {
                error: Error::NIMPL,
                origin: cell,
                message: "Arrays not supported yet".to_string(),
            }),
        }
    }
}

impl Model {
    pub(crate) fn fn_day(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let serial_number = match self.get_number(&args[0], cell) {
            Ok(c) => c.floor() as i64,
            Err(s) => return s,
        };
        let date = match from_excel_date(serial_number) {
            Ok(date) => date,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                }
            }
        };
        let day = date.day() as f64;
        CalcResult::Number(day)
    }

    pub(crate) fn fn_month(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let serial_number = match self.get_number(&args[0], cell) {
            Ok(c) => c.floor() as i64,
            Err(s) => return s,
        };
        let date = match from_excel_date(serial_number) {
            Ok(date) => date,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                }
            }
        };
        let month = date.month() as f64;
        CalcResult::Number(month)
    }

    pub(crate) fn fn_eomonth(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let serial_number = match self.get_number(&args[0], cell) {
            Ok(c) => {
                let t = c.floor() as i64;
                if t < 0 {
                    return CalcResult::Error {
                        error: Error::NUM,
                        origin: cell,
                        message: "Function EOMONTH parameter 1 value is negative. It should be positive or zero.".to_string(),
                    };
                }
                t
            }
            Err(s) => return s,
        };
        let date = match from_excel_date(serial_number) {
            Ok(date) => date,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                }
            }
        };
        if serial_number > MAXIMUM_DATE_SERIAL_NUMBER as i64 {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Function DAY parameter 1 value is too large.".to_string(),
            };
        }

        let months = match self.get_number_no_bools(&args[1], cell) {
            Ok(c) => {
                let t = c.trunc();
                t as i32
            }
            Err(s) => return s,
        };

        let months_abs = months.unsigned_abs();

        let native_date = if months > 0 {
            date + Months::new(months_abs)
        } else {
            date - Months::new(months_abs)
        };

        // Instead of calculating the end of month we compute the first day of the following month
        // and take one day.
        let mut month = native_date.month() + 1;
        let mut year = native_date.year();
        if month == 13 {
            month = 1;
            year += 1;
        }
        match date_to_serial_number(1, month, year) {
            Ok(serial_number) => CalcResult::Number(serial_number as f64 - 1.0),
            Err(message) => CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message,
            },
        }
    }

    // year, month, day
    pub(crate) fn fn_date(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 3 {
            return CalcResult::new_args_number_error(cell);
        }
        let year = match self.get_number(&args[0], cell) {
            Ok(c) => {
                let t = c.floor() as i32;
                if t < 0 {
                    return CalcResult::Error {
                        error: Error::NUM,
                        origin: cell,
                        message: "Out of range parameters for date".to_string(),
                    };
                }
                t
            }
            Err(s) => return s,
        };
        let month = match self.get_number(&args[1], cell) {
            Ok(c) => {
                let t = c.floor();
                t as i32
            }
            Err(s) => return s,
        };
        let day = match self.get_number(&args[2], cell) {
            Ok(c) => {
                let t = c.floor();
                t as i32
            }
            Err(s) => return s,
        };
        match permissive_date_to_serial_number(day, month, year) {
            Ok(serial_number) => CalcResult::Number(serial_number as f64),
            Err(message) => CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message,
            },
        }
    }

    pub(crate) fn fn_year(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let serial_number = match self.get_number(&args[0], cell) {
            Ok(c) => c.floor() as i64,
            Err(s) => return s,
        };
        let date = match from_excel_date(serial_number) {
            Ok(date) => date,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                }
            }
        };
        let year = date.year() as f64;
        CalcResult::Number(year)
    }

    // date, months
    pub(crate) fn fn_edate(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let serial_number = match self.get_number(&args[0], cell) {
            Ok(c) => c.floor() as i64,
            Err(s) => return s,
        };
        let date = match from_excel_date(serial_number) {
            Ok(date) => date,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                }
            }
        };

        let months = match self.get_number(&args[1], cell) {
            Ok(c) => {
                let t = c.trunc();
                t as i32
            }
            Err(s) => return s,
        };

        let months_abs = months.unsigned_abs();

        let native_date = if months > 0 {
            date + Months::new(months_abs)
        } else {
            date - Months::new(months_abs)
        };

        let serial_number = native_date.num_days_from_ce() - EXCEL_DATE_BASE;
        if serial_number < MINIMUM_DATE_SERIAL_NUMBER {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "EDATE out of bounds".to_string(),
            };
        }
        CalcResult::Number(serial_number as f64)
    }

    pub(crate) fn fn_today(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 0 {
            return CalcResult::Error {
                error: Error::ERROR,
                origin: cell,
                message: "Wrong number of arguments".to_string(),
            };
        }
        // milliseconds since January 1, 1970 00:00:00 UTC.
        let milliseconds = get_milliseconds_since_epoch();
        let seconds = milliseconds / 1000;
        let local_time = match DateTime::from_timestamp(seconds, 0) {
            Some(dt) => dt.with_timezone(&self.tz),
            None => {
                return CalcResult::Error {
                    error: Error::ERROR,
                    origin: cell,
                    message: "Invalid date".to_string(),
                }
            }
        };
        // 693_594 is computed as:
        // NaiveDate::from_ymd(1900, 1, 1).num_days_from_ce() - 2
        // The 2 days offset is because of Excel 1900 bug
        let days_from_1900 = local_time.num_days_from_ce() - 693_594;

        CalcResult::Number(days_from_1900 as f64)
    }

    pub(crate) fn fn_now(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 0 {
            return CalcResult::Error {
                error: Error::ERROR,
                origin: cell,
                message: "Wrong number of arguments".to_string(),
            };
        }
        // milliseconds since January 1, 1970 00:00:00 UTC.
        let milliseconds = get_milliseconds_since_epoch();
        let seconds = milliseconds / 1000;
        let local_time = match DateTime::from_timestamp(seconds, 0) {
            Some(dt) => dt.with_timezone(&self.tz),
            None => {
                return CalcResult::Error {
                    error: Error::ERROR,
                    origin: cell,
                    message: "Invalid date".to_string(),
                }
            }
        };
        // 693_594 is computed as:
        // NaiveDate::from_ymd(1900, 1, 1).num_days_from_ce() - 2
        // The 2 days offset is because of Excel 1900 bug
        let days_from_1900 = local_time.num_days_from_ce() - 693_594;
        let days = (local_time.num_seconds_from_midnight() as f64) / (60.0 * 60.0 * 24.0);

        CalcResult::Number(days_from_1900 as f64 + days.fract())
    }

    pub(crate) fn fn_datevalue(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        match self.evaluate_node_in_context(&args[0], cell) {
            CalcResult::String(s) => match parse_datevalue_text(&s) {
                Ok(n) => CalcResult::Number(n as f64),
                Err(_) => CalcResult::Error {
                    error: Error::VALUE,
                    origin: cell,
                    message: "Invalid date".to_string(),
                },
            },
            CalcResult::Number(f) => CalcResult::Number(f.floor()),
            CalcResult::Boolean(b) => {
                if b {
                    CalcResult::Number(1.0)
                } else {
                    CalcResult::Number(0.0)
                }
            }
            err @ CalcResult::Error { .. } => err,
            CalcResult::Range { .. } | CalcResult::Array(_) => CalcResult::Error {
                error: Error::NIMPL,
                origin: cell,
                message: "Arrays not supported yet".to_string(),
            },
            CalcResult::EmptyCell | CalcResult::EmptyArg => CalcResult::Number(0.0),
        }
    }

    pub(crate) fn fn_datedif(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 3 {
            return CalcResult::new_args_number_error(cell);
        }

        let start_serial = match self.get_date_serial(&args[0], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let end_serial = match self.get_date_serial(&args[1], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if end_serial < start_serial {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Start date greater than end date".to_string(),
            };
        }
        let start = match from_excel_date(start_serial) {
            Ok(d) => d,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                }
            }
        };
        let end = match from_excel_date(end_serial) {
            Ok(d) => d,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                }
            }
        };

        let unit = match self.get_string(&args[2], cell) {
            Ok(s) => s.to_ascii_uppercase(),
            Err(e) => return e,
        };

        let result = match unit.as_str() {
            "Y" => {
                let mut years = end.year() - start.year();
                if (end.month(), end.day()) < (start.month(), start.day()) {
                    years -= 1;
                }
                years as f64
            }
            "M" => {
                let mut months =
                    (end.year() - start.year()) * 12 + (end.month() as i32 - start.month() as i32);
                if end.day() < start.day() {
                    months -= 1;
                }
                months as f64
            }
            "D" => (end_serial - start_serial) as f64,
            "YM" => {
                let mut months =
                    (end.year() - start.year()) * 12 + (end.month() as i32 - start.month() as i32);
                if end.day() < start.day() {
                    months -= 1;
                }
                (months % 12).abs() as f64
            }
            "YD" => {
                // Helper to create a date or early-return with #NUM! if impossible
                let make_date = |y: i32, m: u32, d: u32| -> Result<chrono::NaiveDate, CalcResult> {
                    match chrono::NaiveDate::from_ymd_opt(y, m, d) {
                        Some(dt) => Ok(dt),
                        None => Err(CalcResult::Error {
                            error: Error::NUM,
                            origin: cell,
                            message: "Invalid date".to_string(),
                        }),
                    }
                };

                // Compute the last valid day of a given month/year.
                let make_last_day_of_month =
                    |y: i32, m: u32| -> Result<chrono::NaiveDate, CalcResult> {
                        let (next_y, next_m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
                        let first_next = make_date(next_y, next_m, 1)?;
                        let last_day = first_next - chrono::Duration::days(1);
                        make_date(y, m, last_day.day())
                    };

                // Attempt to build the anniversary date in the end year.
                let mut start_adj =
                    match chrono::NaiveDate::from_ymd_opt(end.year(), start.month(), start.day()) {
                        Some(d) => d,
                        None => match make_last_day_of_month(end.year(), start.month()) {
                            Ok(d) => d,
                            Err(e) => return e,
                        },
                    };

                // If the adjusted date is after the end date, shift one year back.
                if start_adj > end {
                    let shift_year = end.year() - 1;
                    start_adj = match chrono::NaiveDate::from_ymd_opt(
                        shift_year,
                        start.month(),
                        start.day(),
                    ) {
                        Some(d) => d,
                        None => match make_last_day_of_month(shift_year, start.month()) {
                            Ok(d) => d,
                            Err(e) => return e,
                        },
                    };
                }

                (end - start_adj).num_days() as f64
            }
            "MD" => {
                let mut months =
                    (end.year() - start.year()) * 12 + (end.month() as i32 - start.month() as i32);
                if end.day() < start.day() {
                    months -= 1;
                }
                let start_shifted = if months >= 0 {
                    start + Months::new(months as u32)
                } else {
                    start - Months::new((-months) as u32)
                };
                (end - start_shifted).num_days() as f64
            }
            _ => {
                return CalcResult::Error {
                    error: Error::VALUE,
                    origin: cell,
                    message: "Invalid unit".to_string(),
                };
            }
        };
        CalcResult::Number(result)
    }
}
