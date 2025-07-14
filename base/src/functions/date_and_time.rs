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

    fn get_array_of_dates(
        &mut self,
        arg: &Node,
        cell: CellReferenceIndex,
    ) -> Result<Vec<i64>, CalcResult> {
        let mut values = Vec::new();
        match self.evaluate_node_in_context(arg, cell) {
            CalcResult::Number(v) => values.push(v.floor() as i64),
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
                            CalcResult::Number(v) => values.push(v.floor() as i64),
                            e @ CalcResult::Error { .. } => return Err(e),
                            _ => {}
                        }
                    }
                }
            }
            e @ CalcResult::Error { .. } => return Err(e),
            _ => {}
        }
        for &v in &values {
            if from_excel_date(v).is_err() {
                return Err(CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                });
            }
        }
        Ok(values)
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
            let values = match self.get_array_of_dates(&args[2], cell) {
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

    fn parse_weekend_pattern(
        &mut self,
        node: Option<&Node>,
        cell: CellReferenceIndex,
    ) -> Result<[bool; 7], CalcResult> {
        let mut weekend = [false, false, false, false, false, true, true];
        if node.is_none() {
            return Ok(weekend);
        }
        match self.evaluate_node_in_context(node.unwrap(), cell) {
            CalcResult::Number(n) => {
                let code = n.trunc() as i32;
                if (n - n.trunc()).abs() > f64::EPSILON {
                    return Err(CalcResult::new_error(
                        Error::VALUE,
                        cell,
                        "Invalid weekend".to_string(),
                    ));
                }
                weekend = match code {
                    1 | 0 => [false, false, false, false, false, true, true],
                    2 => [true, false, false, false, false, false, true],
                    3 => [true, true, false, false, false, false, false],
                    4 => [false, true, true, false, false, false, false],
                    5 => [false, false, true, true, false, false, false],
                    6 => [false, false, false, true, true, false, false],
                    7 => [false, false, false, false, true, true, false],
                    11 => [false, false, false, false, false, false, true],
                    12 => [true, false, false, false, false, false, false],
                    13 => [false, true, false, false, false, false, false],
                    14 => [false, false, true, false, false, false, false],
                    15 => [false, false, false, true, false, false, false],
                    16 => [false, false, false, false, true, false, false],
                    17 => [false, false, false, false, false, true, false],
                    _ => {
                        return Err(CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "Invalid weekend".to_string(),
                        ))
                    }
                };
                Ok(weekend)
            }
            CalcResult::String(s) => {
                if s.len() != 7 || !s.chars().all(|c| c == '0' || c == '1') {
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
                error: Error::NIMPL,
                origin: cell,
                message: "Arrays not supported yet".to_string(),
            }),
            CalcResult::EmptyCell | CalcResult::EmptyArg => Ok(weekend),
            CalcResult::Array(_) => Err(CalcResult::Error {
                error: Error::NIMPL,
                origin: cell,
                message: "Arrays not supported yet".to_string(),
            }),
        }
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

        let weekend_pattern = match self.parse_weekend_pattern(args.get(2), cell) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let mut holidays: std::collections::HashSet<i64> = std::collections::HashSet::new();
        if args.len() == 4 {
            let values = match self.get_array_of_dates(&args[3], cell) {
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
}
