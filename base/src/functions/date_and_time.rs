use chrono::DateTime;
use chrono::Datelike;
use chrono::Months;
use chrono::NaiveDateTime;
use chrono::NaiveTime;
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

fn parse_time_string(text: &str) -> Option<f64> {
    let text = text.trim();

    // First, try custom parsing for edge cases like "24:00:00", "23:60:00", "23:59:60"
    // that need normalization to match Excel behavior
    if let Some(time_fraction) = parse_time_with_normalization(text) {
        return Some(time_fraction);
    }

    // First, try manual parsing for simple "N PM" / "N AM" format
    if let Some((hour_str, is_pm)) = parse_simple_am_pm(text) {
        if let Ok(hour) = hour_str.parse::<u32>() {
            if (1..=12).contains(&hour) {
                let hour_24 = if is_pm {
                    if hour == 12 {
                        12
                    } else {
                        hour + 12
                    }
                } else if hour == 12 {
                    0
                } else {
                    hour
                };
                let time = NaiveTime::from_hms_opt(hour_24, 0, 0)?;
                return Some(time.num_seconds_from_midnight() as f64 / 86_400.0);
            }
        }
    }

    // Standard patterns
    let patterns_time = ["%H:%M:%S", "%H:%M", "%I:%M %p", "%I %p", "%I:%M:%S %p"];
    for p in patterns_time {
        if let Ok(t) = NaiveTime::parse_from_str(text, p) {
            return Some(t.num_seconds_from_midnight() as f64 / 86_400.0);
        }
    }

    let patterns_dt = [
        // ISO formats
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        // Excel-style date formats with AM/PM
        "%d-%b-%Y %I:%M:%S %p", // "22-Aug-2011 6:35:00 AM"
        "%d-%b-%Y %I:%M %p",    // "22-Aug-2011 6:35 AM"
        "%d-%b-%Y %H:%M:%S",    // "22-Aug-2011 06:35:00"
        "%d-%b-%Y %H:%M",       // "22-Aug-2011 06:35"
        // US date formats with AM/PM
        "%m/%d/%Y %I:%M:%S %p", // "8/22/2011 6:35:00 AM"
        "%m/%d/%Y %I:%M %p",    // "8/22/2011 6:35 AM"
        "%m/%d/%Y %H:%M:%S",    // "8/22/2011 06:35:00"
        "%m/%d/%Y %H:%M",       // "8/22/2011 06:35"
        // European date formats with AM/PM
        "%d/%m/%Y %I:%M:%S %p", // "22/8/2011 6:35:00 AM"
        "%d/%m/%Y %I:%M %p",    // "22/8/2011 6:35 AM"
        "%d/%m/%Y %H:%M:%S",    // "22/8/2011 06:35:00"
        "%d/%m/%Y %H:%M",       // "22/8/2011 06:35"
    ];
    for p in patterns_dt {
        if let Ok(dt) = NaiveDateTime::parse_from_str(text, p) {
            return Some(dt.time().num_seconds_from_midnight() as f64 / 86_400.0);
        }
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
        return Some(dt.time().num_seconds_from_midnight() as f64 / 86_400.0);
    }
    None
}

// Custom parser that handles time normalization like Excel does
fn parse_time_with_normalization(text: &str) -> Option<f64> {
    // Try to parse H:M:S format with potential overflow values
    let parts: Vec<&str> = text.split(':').collect();

    if parts.len() == 3 {
        // H:M:S format
        if let (Ok(h), Ok(m), Ok(s)) = (
            parts[0].parse::<i32>(),
            parts[1].parse::<i32>(),
            parts[2].parse::<i32>(),
        ) {
            // Only normalize specific edge cases that Excel handles
            // Don't normalize arbitrary large values like 25:00:00
            if should_normalize_time_components(h, m, s) {
                return Some(normalize_time_components(h, m, s));
            }
        }
    } else if parts.len() == 2 {
        // H:M format (assume seconds = 0)
        if let (Ok(h), Ok(m)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>()) {
            // Only normalize specific edge cases
            if should_normalize_time_components(h, m, 0) {
                return Some(normalize_time_components(h, m, 0));
            }
        }
    }

    None
}

// Normalize time components with overflow handling (like Excel)
fn normalize_time_components(hour: i32, minute: i32, second: i32) -> f64 {
    // Convert everything to total seconds
    let mut total_seconds = hour * 3600 + minute * 60 + second;

    // Handle negative values by wrapping around
    if total_seconds < 0 {
        total_seconds = total_seconds.rem_euclid(86400);
    }

    // Normalize to within a day (0-86399 seconds)
    total_seconds %= 86400;

    // Convert to fraction of a day
    total_seconds as f64 / 86400.0
}

// Check if time components should be normalized (only specific Excel edge cases)
fn should_normalize_time_components(hour: i32, minute: i32, second: i32) -> bool {
    // Only normalize these specific cases that Excel handles:
    // 1. Hour 24 with valid minutes/seconds
    // 2. Hour 23 with minute 60 (becomes 24:00)
    // 3. Any time with second 60 that normalizes to exactly 24:00

    if hour == 24 && (0..=59).contains(&minute) && (0..=59).contains(&second) {
        return true; // 24:MM:SS -> normalize to next day
    }

    if hour == 23 && minute == 60 && (0..=59).contains(&second) {
        return true; // 23:60:SS -> normalize to 24:00:SS
    }

    if (0..=23).contains(&hour) && (0..=59).contains(&minute) && second == 60 {
        // Check if this normalizes to exactly 24:00:00
        let total_seconds = hour * 3600 + minute * 60 + second;
        return total_seconds == 86400; // Exactly 24:00:00
    }

    false
}

// Helper function to parse simple "N PM" / "N AM" formats
fn parse_simple_am_pm(text: &str) -> Option<(&str, bool)> {
    if let Some(hour_part) = text.strip_suffix(" PM") {
        if hour_part.chars().all(|c| c.is_ascii_digit()) {
            return Some((hour_part, true));
        }
    } else if let Some(hour_part) = text.strip_suffix(" AM") {
        if hour_part.chars().all(|c| c.is_ascii_digit()) {
            return Some((hour_part, false));
        }
    }
    None
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

    pub(crate) fn fn_day(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let value = match self.get_number(&args[0], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let days = value.floor() as i32;
        if days < MINIMUM_DATE_SERIAL_NUMBER || days > MAXIMUM_DATE_SERIAL_NUMBER {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Out of range parameters for date".to_string(),
            };
        }
        let date = match from_excel_date(days as i64) {
            Ok(date) => date,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                }
            }
        };
        CalcResult::Number(date.day() as f64)
    }

    pub(crate) fn fn_month(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let value = match self.get_number(&args[0], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let days = value.floor() as i32;
        if days < MINIMUM_DATE_SERIAL_NUMBER || days > MAXIMUM_DATE_SERIAL_NUMBER {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Out of range parameters for date".to_string(),
            };
        }
        let date = match from_excel_date(days as i64) {
            Ok(date) => date,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                }
            }
        };
        CalcResult::Number(date.month() as f64)
    }

    pub(crate) fn fn_year(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let value = match self.get_number(&args[0], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let days = value.floor() as i32;
        if days < MINIMUM_DATE_SERIAL_NUMBER || days > MAXIMUM_DATE_SERIAL_NUMBER {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Out of range parameters for date".to_string(),
            };
        }
        let date = match from_excel_date(days as i64) {
            Ok(date) => date,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                }
            }
        };
        CalcResult::Number(date.year() as f64)
    }

    pub(crate) fn fn_date(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 3 {
            return CalcResult::new_args_number_error(cell);
        }
        let year = match self.get_number(&args[0], cell) {
            Ok(f) => f,
            Err(e) => return e,
        };
        let month = match self.get_number(&args[1], cell) {
            Ok(f) => f,
            Err(e) => return e,
        };
        let day = match self.get_number(&args[2], cell) {
            Ok(f) => f,
            Err(e) => return e,
        };
        match permissive_date_to_serial_number(day as i32, month as i32, year as i32) {
            Ok(n) => CalcResult::Number(n as f64),
            Err(_) => CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Out of range parameters for date".to_string(),
            },
        }
    }

    pub(crate) fn fn_edate(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let start_date = match self.get_number(&args[0], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let months = match self.get_number(&args[1], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let start_days = start_date.floor() as i32;
        if start_days < MINIMUM_DATE_SERIAL_NUMBER || start_days > MAXIMUM_DATE_SERIAL_NUMBER {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Out of range parameters for date".to_string(),
            };
        }
        let start_date = match from_excel_date(start_days as i64) {
            Ok(date) => date,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                }
            }
        };
        let months = months.floor() as i32;
        let result_date = if months >= 0 {
            start_date + Months::new(months as u32)
        } else {
            start_date - Months::new((-months) as u32)
        };
        let serial_number = result_date.num_days_from_ce() - EXCEL_DATE_BASE;
        CalcResult::Number(serial_number as f64)
    }

    pub(crate) fn fn_eomonth(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let start_date = match self.get_number(&args[0], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let months = match self.get_number(&args[1], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let start_days = start_date.floor() as i32;
        if start_days < MINIMUM_DATE_SERIAL_NUMBER || start_days > MAXIMUM_DATE_SERIAL_NUMBER {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Out of range parameters for date".to_string(),
            };
        }
        let start_date = match from_excel_date(start_days as i64) {
            Ok(date) => date,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                }
            }
        };
        let months = months.floor() as i32;
        let result_date = if months >= 0 {
            start_date + Months::new(months as u32 + 1)
        } else {
            start_date - Months::new((-months) as u32 - 1)
        };
        let last_day_of_month = result_date.with_day(1).unwrap() - chrono::Duration::days(1);
        let serial_number = last_day_of_month.num_days_from_ce() - EXCEL_DATE_BASE;
        CalcResult::Number(serial_number as f64)
    }

    pub(crate) fn fn_now(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 0 {
            return CalcResult::new_args_number_error(cell);
        }
        let milliseconds = get_milliseconds_since_epoch();
        let days = (milliseconds as f64) / (24.0 * 60.0 * 60.0 * 1000.0);
        let days_from_1900 = days + 25569.0;

        CalcResult::Number(days_from_1900)
    }

    pub(crate) fn fn_today(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let args_count = args.len();
        if args_count != 0 {
            return CalcResult::new_args_number_error(cell);
        }
        let milliseconds = get_milliseconds_since_epoch();
        let days = ((milliseconds as f64) / (24.0 * 60.0 * 60.0 * 1000.0)).floor();
        let days_from_1900 = days + 25569.0;

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

    pub(crate) fn fn_time(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 3 {
            return CalcResult::new_args_number_error(cell);
        }
        let hour = match self.get_number(&args[0], cell) {
            Ok(f) => f,
            Err(e) => return e,
        };
        let minute = match self.get_number(&args[1], cell) {
            Ok(f) => f,
            Err(e) => return e,
        };
        let second = match self.get_number(&args[2], cell) {
            Ok(f) => f,
            Err(e) => return e,
        };
        if hour < 0.0 || minute < 0.0 || second < 0.0 {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Invalid time".to_string(),
            };
        }
        let total_seconds = hour.floor() * 3600.0 + minute.floor() * 60.0 + second.floor();
        let day_seconds = 24.0 * 3600.0;
        let secs = total_seconds.rem_euclid(day_seconds);
        CalcResult::Number(secs / day_seconds)
    }

    pub(crate) fn fn_timevalue(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let text = match self.get_string(&args[0], cell) {
            Ok(s) => s,
            Err(e) => return e,
        };
        match parse_time_string(&text) {
            Some(value) => CalcResult::Number(value),
            None => CalcResult::Error {
                error: Error::VALUE,
                origin: cell,
                message: "Invalid time".to_string(),
            },
        }
    }

    pub(crate) fn fn_hour(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let value = match self.get_number(&args[0], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if value < 0.0 {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Invalid time".to_string(),
            };
        }
        let hours = (value.rem_euclid(1.0) * 24.0).floor();
        CalcResult::Number(hours)
    }

    pub(crate) fn fn_minute(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let value = match self.get_number(&args[0], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if value < 0.0 {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Invalid time".to_string(),
            };
        }
        let total_seconds = (value.rem_euclid(1.0) * 86400.0).floor();
        let minutes = ((total_seconds / 60.0) as i64 % 60) as f64;
        CalcResult::Number(minutes)
    }

    pub(crate) fn fn_second(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let value = match self.get_number(&args[0], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };
        if value < 0.0 {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Invalid time".to_string(),
            };
        }
        let total_seconds = (value.rem_euclid(1.0) * 86400.0).floor();
        let seconds = (total_seconds as i64 % 60) as f64;
        CalcResult::Number(seconds)
    }

    pub(crate) fn fn_days(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let (end_serial, _end_date) = match self.get_and_validate_date_serial(&args[0], cell) {
            Ok((s, d)) => (s, d),
            Err(e) => return e,
        };
        let (start_serial, _start_date) = match self.get_and_validate_date_serial(&args[1], cell) {
            Ok((s, d)) => (s, d),
            Err(e) => return e,
        };
        CalcResult::Number((end_serial - start_serial) as f64)
    }

    pub(crate) fn fn_days360(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(2..=3).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let (_start_serial, start_date) = match self.get_and_validate_date_serial(&args[0], cell) {
            Ok((s, d)) => (s, d),
            Err(e) => return e,
        };
        let (_end_serial, end_date) = match self.get_and_validate_date_serial(&args[1], cell) {
            Ok((s, d)) => (s, d),
            Err(e) => return e,
        };
        let method = if args.len() == 3 {
            match self.get_number(&args[2], cell) {
                Ok(f) => f != 0.0,
                Err(s) => return s,
            }
        } else {
            false
        };

        fn last_day_feb(year: i32) -> u32 {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        let mut sd_day = start_date.day();
        let sd_month = start_date.month();
        let sd_year = start_date.year();
        let mut ed_day = end_date.day();
        let ed_month = end_date.month();
        let ed_year = end_date.year();

        if method {
            if sd_day == 31 {
                sd_day = 30;
            }
            if ed_day == 31 {
                ed_day = 30;
            }
        } else {
            if (sd_month == 2 && sd_day == last_day_feb(sd_year)) || sd_day == 31 {
                sd_day = 30;
            }
            if ed_month == 2 && ed_day == last_day_feb(ed_year) && sd_day == 30 {
                ed_day = 30;
            }
            if ed_day == 31 && sd_day >= 30 {
                ed_day = 30;
            }
        }

        let result = (ed_year - sd_year) * 360
            + (ed_month as i32 - sd_month as i32) * 30
            + (ed_day as i32 - sd_day as i32);
        CalcResult::Number(result as f64)
    }

    pub(crate) fn fn_weekday(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(1..=2).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let (_serial, date) = match self.get_and_validate_date_serial(&args[0], cell) {
            Ok((s, d)) => (s, d),
            Err(e) => return e,
        };
        let return_type = if args.len() == 2 {
            match self.get_number(&args[1], cell) {
                Ok(f) => f as i32,
                Err(s) => return s,
            }
        } else {
            1
        };
        let weekday = date.weekday();
        let num = match return_type {
            1 => weekday.num_days_from_sunday() + 1,
            2 => weekday.number_from_monday(),
            3 => (weekday.number_from_monday() - 1) % 7, // 0-based Monday start
            11..=17 => {
                let start = (return_type - 11) as u32; // 0 for Monday
                ((weekday.number_from_monday() + 7 - start) % 7) + 1
            }
            0 => {
                return CalcResult::new_error(Error::VALUE, cell, "Invalid return_type".to_string())
            }
            _ => return CalcResult::new_error(Error::NUM, cell, "Invalid return_type".to_string()),
        } as u32;
        CalcResult::Number(num as f64)
    }

    pub(crate) fn fn_weeknum(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(1..=2).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let (_serial, date) = match self.get_and_validate_date_serial(&args[0], cell) {
            Ok((s, d)) => (s, d),
            Err(e) => return e,
        };
        let return_type = if args.len() == 2 {
            match self.get_number(&args[1], cell) {
                Ok(f) => f as i32,
                Err(s) => return s,
            }
        } else {
            1
        };
        if return_type == 21 {
            let w = date.iso_week().week();
            return CalcResult::Number(w as f64);
        }
        let start_offset = match return_type {
            1 => chrono::Weekday::Sun,
            2 | 11 => chrono::Weekday::Mon,
            12 => chrono::Weekday::Tue,
            13 => chrono::Weekday::Wed,
            14 => chrono::Weekday::Thu,
            15 => chrono::Weekday::Fri,
            16 => chrono::Weekday::Sat,
            17 => chrono::Weekday::Sun,
            x if x <= 0 || x == 3 => {
                return CalcResult::new_error(Error::VALUE, cell, "Invalid return_type".to_string())
            }
            _ => return CalcResult::new_error(Error::NUM, cell, "Invalid return_type".to_string()),
        };
        let mut first = match chrono::NaiveDate::from_ymd_opt(date.year(), 1, 1) {
            Some(d) => d,
            None => {
                return CalcResult::new_error(
                    Error::NUM,
                    cell,
                    "Out of range parameters for date".to_string(),
                );
            }
        };
        while first.weekday() != start_offset {
            first -= chrono::Duration::days(1);
        }
        let week = ((date - first).num_days() / 7 + 1) as i64;
        CalcResult::Number(week as f64)
    }

    pub(crate) fn fn_isoweeknum(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let (_serial, date) = match self.get_and_validate_date_serial(&args[0], cell) {
            Ok((s, d)) => (s, d),
            Err(e) => return e,
        };
        CalcResult::Number(date.iso_week().week() as f64)
    }

    fn is_weekend(day: chrono::Weekday, weekend_mask: &[bool; 7]) -> bool {
        match day {
            chrono::Weekday::Mon => weekend_mask[0],
            chrono::Weekday::Tue => weekend_mask[1],
            chrono::Weekday::Wed => weekend_mask[2],
            chrono::Weekday::Thu => weekend_mask[3],
            chrono::Weekday::Fri => weekend_mask[4],
            chrono::Weekday::Sat => weekend_mask[5],
            chrono::Weekday::Sun => weekend_mask[6],
        }
    }

    pub(crate) fn fn_workday(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(2..=3).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let (_start_serial, mut date) = match self.get_and_validate_date_serial(&args[0], cell) {
            Ok((s, d)) => (s, d),
            Err(e) => return e,
        };
        let mut days = match self.get_number(&args[1], cell) {
            Ok(f) => f as i32,
            Err(s) => return s,
        };
        let weekend = [false, false, false, false, false, true, true];
        let holidays = match self.process_date_array(args.get(2), cell) {
            Ok(dates) => self.dates_to_holiday_set(dates),
            Err(e) => return e,
        };
        while days != 0 {
            if days > 0 {
                date += chrono::Duration::days(1);
                if !Self::is_weekend(date.weekday(), &weekend) && !holidays.contains(&date) {
                    days -= 1;
                }
            } else {
                date -= chrono::Duration::days(1);
                if !Self::is_weekend(date.weekday(), &weekend) && !holidays.contains(&date) {
                    days += 1;
                }
            }
        }
        let serial = date.num_days_from_ce() - EXCEL_DATE_BASE;
        CalcResult::Number(serial as f64)
    }

    // Consolidated holiday/date array processing function
    fn process_date_array(
        &mut self,
        arg_option: Option<&Node>,
        cell: CellReferenceIndex,
    ) -> Result<Vec<i64>, CalcResult> {
        let mut values = Vec::new();

        if let Some(arg) = arg_option {
            match self.evaluate_node_in_context(arg, cell) {
                CalcResult::Number(value) => {
                    let serial = value.floor() as i64;
                    if from_excel_date(serial).is_err() {
                        return Err(CalcResult::Error {
                            error: Error::NUM,
                            origin: cell,
                            message: "Out of range parameters for date".to_string(),
                        });
                    }
                    values.push(serial);
                }
                CalcResult::Range { left, right } => {
                    if left.sheet != right.sheet {
                        return Err(CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "Ranges are in different sheets".to_string(),
                        ));
                    }
                    for row in left.row..=right.row {
                        for column in left.column..=right.column {
                            match self.evaluate_cell(CellReferenceIndex {
                                sheet: left.sheet,
                                row,
                                column,
                            }) {
                                CalcResult::Number(value) => {
                                    let serial = value.floor() as i64;
                                    if from_excel_date(serial).is_err() {
                                        return Err(CalcResult::Error {
                                            error: Error::NUM,
                                            origin: cell,
                                            message: "Out of range parameters for date".to_string(),
                                        });
                                    }
                                    values.push(serial);
                                }
                                CalcResult::EmptyCell => {
                                    // Empty cells are ignored in holiday lists
                                }
                                e @ CalcResult::Error { .. } => return Err(e),
                                _ => {
                                    // Non-numeric values in holiday lists should cause VALUE error
                                    return Err(CalcResult::Error {
                                        error: Error::VALUE,
                                        origin: cell,
                                        message: "Invalid holiday date".to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
                CalcResult::Array(array) => {
                    for row in &array {
                        for value in row {
                            match value {
                                crate::expressions::parser::ArrayNode::Number(n) => {
                                    let serial = n.floor() as i64;
                                    if from_excel_date(serial).is_err() {
                                        return Err(CalcResult::Error {
                                            error: Error::NUM,
                                            origin: cell,
                                            message: "Out of range parameters for date".to_string(),
                                        });
                                    }
                                    values.push(serial);
                                }
                                _ => {
                                    return Err(CalcResult::Error {
                                        error: Error::VALUE,
                                        origin: cell,
                                        message: "Invalid holiday date".to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
                CalcResult::String(_) => {
                    // String holidays should cause VALUE error
                    return Err(CalcResult::Error {
                        error: Error::VALUE,
                        origin: cell,
                        message: "Invalid holiday date".to_string(),
                    });
                }
                CalcResult::EmptyCell | CalcResult::EmptyArg => {}
                e @ CalcResult::Error { .. } => return Err(e),
                _ => {
                    // Other non-numeric types should cause VALUE error
                    return Err(CalcResult::Error {
                        error: Error::VALUE,
                        origin: cell,
                        message: "Invalid holiday date".to_string(),
                    });
                }
            }
        }
        Ok(values)
    }

    // Helper to convert Vec<i64> to HashSet<NaiveDate> for backward compatibility
    fn dates_to_holiday_set(
        &self,
        dates: Vec<i64>,
    ) -> std::collections::HashSet<chrono::NaiveDate> {
        dates
            .into_iter()
            .filter_map(|serial| from_excel_date(serial).ok())
            .collect()
    }

    // Consolidated date validation helper - eliminates 29+ duplicate patterns
    fn validate_and_convert_date_serial(
        &self,
        serial: i64,
        cell: CellReferenceIndex,
    ) -> Result<chrono::NaiveDate, CalcResult> {
        match from_excel_date(serial) {
            Ok(date) => Ok(date),
            Err(_) => Err(CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Out of range parameters for date".to_string(),
            }),
        }
    }

    // Helper for common pattern: get number, floor to i64, validate as date
    fn get_and_validate_date_serial(
        &mut self,
        arg: &Node,
        cell: CellReferenceIndex,
    ) -> Result<(i64, chrono::NaiveDate), CalcResult> {
        let serial = match self.get_number(arg, cell) {
            Ok(n) => n.floor() as i64,
            Err(e) => return Err(e),
        };
        let date = self.validate_and_convert_date_serial(serial, cell)?;
        Ok((serial, date))
    }

    // Consolidated weekend pattern processing function
    fn parse_weekend_pattern_unified(
        &mut self,
        node: Option<&Node>,
        cell: CellReferenceIndex,
    ) -> Result<[bool; 7], CalcResult> {
        // Default: Saturday-Sunday weekend (pattern 1)
        let mut weekend = [false, false, false, false, false, true, true];

        if let Some(node_ref) = node {
            match self.evaluate_node_in_context(node_ref, cell) {
                CalcResult::Number(n) => {
                    let code = n.trunc() as i32;
                    if (n - n.trunc()).abs() > f64::EPSILON {
                        return Err(CalcResult::new_error(
                            Error::NUM,
                            cell,
                            "Invalid weekend".to_string(),
                        ));
                    }
                    weekend = match code {
                        1 | 0 => [false, false, false, false, false, true, true], // Saturday-Sunday
                        2 => [true, false, false, false, false, false, true],     // Sunday-Monday
                        3 => [true, true, false, false, false, false, false],     // Monday-Tuesday
                        4 => [false, true, true, false, false, false, false], // Tuesday-Wednesday
                        5 => [false, false, true, true, false, false, false], // Wednesday-Thursday
                        6 => [false, false, false, true, true, false, false], // Thursday-Friday
                        7 => [false, false, false, false, true, true, false], // Friday-Saturday
                        11 => [false, false, false, false, false, false, true], // Sunday only
                        12 => [true, false, false, false, false, false, false], // Monday only
                        13 => [false, true, false, false, false, false, false], // Tuesday only
                        14 => [false, false, true, false, false, false, false], // Wednesday only
                        15 => [false, false, false, true, false, false, false], // Thursday only
                        16 => [false, false, false, false, true, false, false], // Friday only
                        17 => [false, false, false, false, false, true, false], // Saturday only
                        _ => {
                            return Err(CalcResult::new_error(
                                Error::NUM,
                                cell,
                                "Invalid weekend".to_string(),
                            ))
                        }
                    };
                    Ok(weekend)
                }
                CalcResult::String(s) => {
                    if s.len() != 7 {
                        return Err(CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "Invalid weekend".to_string(),
                        ));
                    }
                    if !s.chars().all(|c| c == '0' || c == '1') {
                        return Err(CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "Invalid weekend".to_string(),
                        ));
                    }
                    weekend = [false; 7];
                    for (i, ch) in s.chars().enumerate() {
                        weekend[i] = ch == '1';
                    }
                    Ok(weekend)
                }
                CalcResult::Boolean(_) => Err(CalcResult::new_error(
                    Error::VALUE,
                    cell,
                    "Invalid weekend".to_string(),
                )),
                e @ CalcResult::Error { .. } => Err(e),
                CalcResult::Range { .. } => Err(CalcResult::Error {
                    error: Error::VALUE,
                    origin: cell,
                    message: "Invalid weekend".to_string(),
                }),
                CalcResult::EmptyCell | CalcResult::EmptyArg => Ok(weekend),
                CalcResult::Array(_) => Err(CalcResult::Error {
                    error: Error::VALUE,
                    origin: cell,
                    message: "Invalid weekend".to_string(),
                }),
            }
        } else {
            Ok(weekend)
        }
    }

    // Legacy wrapper for weekend_from_arg (used by WORKDAY.INTL)
    fn weekend_from_arg(
        &mut self,
        arg: Option<&Node>,
        cell: CellReferenceIndex,
    ) -> Result<[bool; 7], CalcResult> {
        self.parse_weekend_pattern_unified(arg, cell)
    }

    pub(crate) fn fn_workday_intl(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        if !(2..=4).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let (_start_serial, mut date) = match self.get_and_validate_date_serial(&args[0], cell) {
            Ok((s, d)) => (s, d),
            Err(e) => return e,
        };
        let mut days = match self.get_number(&args[1], cell) {
            Ok(f) => f as i32,
            Err(s) => return s,
        };
        let weekend_mask = match self.weekend_from_arg(args.get(2), cell) {
            Ok(m) => m,
            Err(e) => return e,
        };
        let holidays = match self.process_date_array(args.get(3), cell) {
            Ok(dates) => self.dates_to_holiday_set(dates),
            Err(e) => return e,
        };
        while days != 0 {
            if days > 0 {
                date += chrono::Duration::days(1);
                if !Self::is_weekend(date.weekday(), &weekend_mask) && !holidays.contains(&date) {
                    days -= 1;
                }
            } else {
                date -= chrono::Duration::days(1);
                if !Self::is_weekend(date.weekday(), &weekend_mask) && !holidays.contains(&date) {
                    days += 1;
                }
            }
        }
        let serial = date.num_days_from_ce() - EXCEL_DATE_BASE;
        CalcResult::Number(serial as f64)
    }

    pub(crate) fn fn_yearfrac(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(2..=3).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let (_start_serial, start_date) = match self.get_and_validate_date_serial(&args[0], cell) {
            Ok((s, d)) => (s, d),
            Err(e) => return e,
        };
        let (_end_serial, end_date) = match self.get_and_validate_date_serial(&args[1], cell) {
            Ok((s, d)) => (s, d),
            Err(e) => return e,
        };
        let basis = if args.len() == 3 {
            match self.get_number(&args[2], cell) {
                Ok(f) => f as i32,
                Err(s) => return s,
            }
        } else {
            0
        };
        let days = (end_date - start_date).num_days() as f64;
        let result = match basis {
            0 => {
                let d360 = self.fn_days360(args, cell);
                if let CalcResult::Number(n) = d360 {
                    n / 360.0
                } else {
                    return d360;
                }
            }
            1 => {
                let year_days = if start_date.year() == end_date.year() {
                    if (start_date.year() % 4 == 0 && start_date.year() % 100 != 0)
                        || start_date.year() % 400 == 0
                    {
                        366.0
                    } else {
                        365.0
                    }
                } else {
                    365.0
                };
                days / year_days
            }
            2 => days / 360.0,
            3 => days / 365.0,
            4 => {
                let d360 = self.fn_days360(
                    &[args[0].clone(), args[1].clone(), Node::NumberKind(1.0)],
                    cell,
                );
                if let CalcResult::Number(n) = d360 {
                    n / 360.0
                } else {
                    return d360;
                }
            }
            _ => return CalcResult::new_error(Error::NUM, cell, "Invalid basis".to_string()),
        };
        CalcResult::Number(result)
    }

    pub(crate) fn fn_networkdays(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(2..=3).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let start_serial = match self.get_number(&args[0], cell) {
            Ok(n) => n.floor() as i64,
            Err(e) => return e,
        };
        let end_serial = match self.get_number(&args[1], cell) {
            Ok(n) => n.floor() as i64,
            Err(e) => return e,
        };
        let mut holidays: std::collections::HashSet<i64> = std::collections::HashSet::new();
        if args.len() == 3 {
            let values = match self.process_date_array(Some(&args[2]), cell) {
                Ok(v) => v,
                Err(e) => return e,
            };
            for v in values {
                holidays.insert(v);
            }
        }

        let (from, to, sign) = if start_serial <= end_serial {
            (start_serial, end_serial, 1.0)
        } else {
            (end_serial, start_serial, -1.0)
        };
        let mut count = 0i64;
        for serial in from..=to {
            let date = match from_excel_date(serial) {
                Ok(d) => d,
                Err(_) => {
                    return CalcResult::Error {
                        error: Error::NUM,
                        origin: cell,
                        message: "Out of range parameters for date".to_string(),
                    }
                }
            };
            let weekday = date.weekday().number_from_monday();
            let is_weekend = matches!(weekday, 6 | 7);
            if !is_weekend && !holidays.contains(&serial) {
                count += 1;
            }
        }
        CalcResult::Number(count as f64 * sign)
    }

    pub(crate) fn fn_networkdays_intl(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        if !(2..=4).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let start_serial = match self.get_number(&args[0], cell) {
            Ok(n) => n.floor() as i64,
            Err(e) => return e,
        };
        let end_serial = match self.get_number(&args[1], cell) {
            Ok(n) => n.floor() as i64,
            Err(e) => return e,
        };

        let weekend_pattern = match self.parse_weekend_pattern_unified(args.get(2), cell) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let mut holidays: std::collections::HashSet<i64> = std::collections::HashSet::new();
        if args.len() == 4 {
            let values = match self.process_date_array(Some(&args[3]), cell) {
                Ok(v) => v,
                Err(e) => return e,
            };
            for v in values {
                holidays.insert(v);
            }
        }

        let (from, to, sign) = if start_serial <= end_serial {
            (start_serial, end_serial, 1.0)
        } else {
            (end_serial, start_serial, -1.0)
        };
        let mut count = 0i64;
        for serial in from..=to {
            let date = match from_excel_date(serial) {
                Ok(d) => d,
                Err(_) => {
                    return CalcResult::Error {
                        error: Error::NUM,
                        origin: cell,
                        message: "Out of range parameters for date".to_string(),
                    }
                }
            };
            let weekday = date.weekday().number_from_monday() as usize - 1;
            if !weekend_pattern[weekday] && !holidays.contains(&serial) {
                count += 1;
            }
        }
        CalcResult::Number(count as f64 * sign)
    }
}
