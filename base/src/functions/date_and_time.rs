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
    calc_result::CalcResult,
    constants::EXCEL_DATE_BASE,
    expressions::parser::{ArrayNode, Node},
    expressions::token::Error,
    formatter::dates::from_excel_date,
    model::Model,
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

    pub(crate) fn fn_days(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let end_serial = match self.get_number(&args[0], cell) {
            Ok(c) => c.floor() as i64,
            Err(s) => return s,
        };
        let start_serial = match self.get_number(&args[1], cell) {
            Ok(c) => c.floor() as i64,
            Err(s) => return s,
        };
        if from_excel_date(end_serial).is_err() || from_excel_date(start_serial).is_err() {
            return CalcResult::Error {
                error: Error::NUM,
                origin: cell,
                message: "Out of range parameters for date".to_string(),
            };
        }
        CalcResult::Number((end_serial - start_serial) as f64)
    }

    pub(crate) fn fn_days360(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if !(2..=3).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let start_serial = match self.get_number(&args[0], cell) {
            Ok(c) => c.floor() as i64,
            Err(s) => return s,
        };
        let end_serial = match self.get_number(&args[1], cell) {
            Ok(c) => c.floor() as i64,
            Err(s) => return s,
        };
        let method = if args.len() == 3 {
            match self.get_number(&args[2], cell) {
                Ok(f) => f != 0.0,
                Err(s) => return s,
            }
        } else {
            false
        };
        let start_date = match from_excel_date(start_serial) {
            Ok(d) => d,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                };
            }
        };
        let end_date = match from_excel_date(end_serial) {
            Ok(d) => d,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                };
            }
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
        let serial = match self.get_number(&args[0], cell) {
            Ok(c) => c.floor() as i64,
            Err(s) => return s,
        };
        let date = match from_excel_date(serial) {
            Ok(d) => d,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                };
            }
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
        let serial = match self.get_number(&args[0], cell) {
            Ok(c) => c.floor() as i64,
            Err(s) => return s,
        };
        let date = match from_excel_date(serial) {
            Ok(d) => d,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                };
            }
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
        let serial = match self.get_number(&args[0], cell) {
            Ok(c) => c.floor() as i64,
            Err(s) => return s,
        };
        let date = match from_excel_date(serial) {
            Ok(d) => d,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                };
            }
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
        let start_serial = match self.get_number(&args[0], cell) {
            Ok(c) => c.floor() as i64,
            Err(s) => return s,
        };
        let mut date = match from_excel_date(start_serial) {
            Ok(d) => d,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                };
            }
        };
        let mut days = match self.get_number(&args[1], cell) {
            Ok(f) => f as i32,
            Err(s) => return s,
        };
        let weekend = [false, false, false, false, false, true, true];
        let holiday_set = match self.get_holiday_set(args.get(2), cell) {
            Ok(h) => h,
            Err(e) => return e,
        };
        while days != 0 {
            if days > 0 {
                date += chrono::Duration::days(1);
                if !Self::is_weekend(date.weekday(), &weekend) && !holiday_set.contains(&date) {
                    days -= 1;
                }
            } else {
                date -= chrono::Duration::days(1);
                if !Self::is_weekend(date.weekday(), &weekend) && !holiday_set.contains(&date) {
                    days += 1;
                }
            }
        }
        let serial = date.num_days_from_ce() - EXCEL_DATE_BASE;
        CalcResult::Number(serial as f64)
    }

    fn get_holiday_set(
        &mut self,
        arg_option: Option<&Node>,
        cell: CellReferenceIndex,
    ) -> Result<std::collections::HashSet<chrono::NaiveDate>, CalcResult> {
        let mut holiday_set = std::collections::HashSet::new();

        if let Some(arg) = arg_option {
            match self.evaluate_node_in_context(arg, cell) {
                CalcResult::Number(value) => {
                    let serial = value.floor() as i64;
                    match from_excel_date(serial) {
                        Ok(date) => {
                            holiday_set.insert(date);
                        }
                        Err(_) => {
                            return Err(CalcResult::Error {
                                error: Error::NUM,
                                origin: cell,
                                message: "Invalid holiday date".to_string(),
                            });
                        }
                    }
                }
                CalcResult::Range { left, right } => {
                    let sheet = left.sheet;
                    for row in left.row..=right.row {
                        for column in left.column..=right.column {
                            let cell_ref = CellReferenceIndex { sheet, row, column };
                            match self.evaluate_cell(cell_ref) {
                                CalcResult::Number(value) => {
                                    let serial = value.floor() as i64;
                                    match from_excel_date(serial) {
                                        Ok(date) => {
                                            holiday_set.insert(date);
                                        }
                                        Err(_) => {
                                            return Err(CalcResult::Error {
                                                error: Error::NUM,
                                                origin: cell,
                                                message: "Invalid holiday date".to_string(),
                                            });
                                        }
                                    }
                                }
                                CalcResult::EmptyCell => {
                                    // Ignore empty cells
                                }
                                CalcResult::Error { .. } => {
                                    // Propagate errors
                                    return Err(CalcResult::Error {
                                        error: Error::VALUE,
                                        origin: cell,
                                        message: "Error in holiday date".to_string(),
                                    });
                                }
                                _ => {
                                    // Ignore non-numeric values
                                }
                            }
                        }
                    }
                }
                CalcResult::Array(array) => {
                    for row in array {
                        for value in row {
                            match value {
                                ArrayNode::Number(num) => {
                                    let serial = num.floor() as i64;
                                    match from_excel_date(serial) {
                                        Ok(date) => {
                                            holiday_set.insert(date);
                                        }
                                        Err(_) => {
                                            return Err(CalcResult::Error {
                                                error: Error::NUM,
                                                origin: cell,
                                                message: "Invalid holiday date".to_string(),
                                            });
                                        }
                                    }
                                }
                                ArrayNode::Error(error) => {
                                    return Err(CalcResult::Error {
                                        error,
                                        origin: cell,
                                        message: "Error in holiday array".to_string(),
                                    });
                                }
                                _ => {
                                    // Ignore non-numeric values
                                }
                            }
                        }
                    }
                }
                error @ CalcResult::Error { .. } => return Err(error),
                _ => {
                    // Ignore other types
                }
            }
        }

        Ok(holiday_set)
    }

    fn weekend_from_arg(
        &mut self,
        arg: Option<&Node>,
        cell: CellReferenceIndex,
    ) -> Result<[bool; 7], CalcResult> {
        if let Some(node) = arg {
            match self.evaluate_node_in_context(node, cell) {
                CalcResult::Number(n) => {
                    let code = n as i32;
                    let mask = match code {
                        1 => [false, false, false, false, false, true, true],
                        2 => [true, false, false, false, false, true, false],
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
                                Error::NUM,
                                cell,
                                "Invalid weekend".to_string(),
                            ))
                        }
                    };
                    Ok(mask)
                }
                CalcResult::String(s) => {
                    let bytes = s.as_bytes();
                    if bytes.len() == 7 && bytes.iter().all(|c| *c == b'0' || *c == b'1') {
                        let mut mask = [false; 7];
                        for (i, b) in bytes.iter().enumerate() {
                            mask[i] = *b == b'1';
                        }
                        Ok(mask)
                    } else {
                        Err(CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "Invalid weekend".to_string(),
                        ))
                    }
                }
                e @ CalcResult::Error { .. } => Err(e),
                _ => Err(CalcResult::new_error(
                    Error::VALUE,
                    cell,
                    "Invalid weekend".to_string(),
                )),
            }
        } else {
            Ok([false, false, false, false, false, true, true])
        }
    }

    pub(crate) fn fn_workday_intl(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        if !(2..=4).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let start_serial = match self.get_number(&args[0], cell) {
            Ok(c) => c.floor() as i64,
            Err(s) => return s,
        };
        let mut date = match from_excel_date(start_serial) {
            Ok(d) => d,
            Err(_) => {
                return CalcResult::Error {
                    error: Error::NUM,
                    origin: cell,
                    message: "Out of range parameters for date".to_string(),
                };
            }
        };
        let mut days = match self.get_number(&args[1], cell) {
            Ok(f) => f as i32,
            Err(s) => return s,
        };
        let weekend_mask = match self.weekend_from_arg(args.get(2), cell) {
            Ok(m) => m,
            Err(e) => return e,
        };
        let holiday_set = match self.get_holiday_set(args.get(3), cell) {
            Ok(h) => h,
            Err(e) => return e,
        };
        while days != 0 {
            if days > 0 {
                date += chrono::Duration::days(1);
                if !Self::is_weekend(date.weekday(), &weekend_mask) && !holiday_set.contains(&date)
                {
                    days -= 1;
                }
            } else {
                date -= chrono::Duration::days(1);
                if !Self::is_weekend(date.weekday(), &weekend_mask) && !holiday_set.contains(&date)
                {
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
        let start_serial = match self.get_number(&args[0], cell) {
            Ok(c) => c.floor() as i64,
            Err(s) => return s,
        };
        let end_serial = match self.get_number(&args[1], cell) {
            Ok(c) => c.floor() as i64,
            Err(s) => return s,
        };
        let basis = if args.len() == 3 {
            match self.get_number(&args[2], cell) {
                Ok(f) => f as i32,
                Err(s) => return s,
            }
        } else {
            0
        };
        let start_date = match from_excel_date(start_serial) {
            Ok(d) => d,
            Err(_) => {
                return CalcResult::new_error(
                    Error::NUM,
                    cell,
                    "Out of range parameters for date".to_string(),
                )
            }
        };
        let end_date = match from_excel_date(end_serial) {
            Ok(d) => d,
            Err(_) => {
                return CalcResult::new_error(
                    Error::NUM,
                    cell,
                    "Out of range parameters for date".to_string(),
                )
            }
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
}
