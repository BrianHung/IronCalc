use std::cmp::Ordering;

use crate::expressions::types::CellReferenceIndex;
use crate::{
    calc_result::CalcResult, expressions::parser::Node, expressions::token::Error, model::Model,
};

impl<'a> Model<'a> {
    // Helper to collect numeric values for PERCENTILE/PERCENTRANK functions
    fn collect_percentile_values(
        &mut self,
        arg: &Node,
        cell: CellReferenceIndex,
    ) -> Result<Vec<f64>, CalcResult> {
        let values = match self.evaluate_node_in_context(arg, cell) {
            CalcResult::Number(value) => vec![Some(value)],
            CalcResult::Boolean(b) => {
                if !matches!(arg, Node::ReferenceKind { .. }) {
                    vec![Some(if b { 1.0 } else { 0.0 })]
                } else {
                    vec![]
                }
            }
            CalcResult::String(s) => {
                if !matches!(arg, Node::ReferenceKind { .. }) {
                    if let Ok(v) = s.parse::<f64>() {
                        vec![Some(v)]
                    } else {
                        return Err(CalcResult::new_error(
                            Error::VALUE,
                            cell,
                            "Argument cannot be cast into number".to_string(),
                        ));
                    }
                } else {
                    vec![]
                }
            }
            CalcResult::Range { left, right } => self.values_from_range(left, right)?,
            CalcResult::Array(array) => match self.values_from_array(array) {
                Ok(v) => v,
                Err(e) => {
                    return Err(CalcResult::Error {
                        error: Error::VALUE,
                        origin: cell,
                        message: format!("Unsupported array argument: {}", e),
                    })
                }
            },
            CalcResult::Error { .. } => return Err(self.evaluate_node_in_context(arg, cell)),
            CalcResult::EmptyCell | CalcResult::EmptyArg => vec![],
        };

        let numeric_values: Vec<f64> = values.into_iter().flatten().collect();
        Ok(numeric_values)
    }

    /// PERCENTILE.INC(array, k)
    /// Returns the k-th percentile of values in a range, where k is in [0,1].
    pub(crate) fn fn_percentile_inc(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }

        let mut values = match self.collect_percentile_values(&args[0], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };

        if values.is_empty() {
            return CalcResult::new_error(Error::NUM, cell, "Empty array".to_string());
        }

        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

        let k = match self.get_number(&args[1], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };

        if !(0.0..=1.0).contains(&k) {
            return CalcResult::new_error(
                Error::NUM,
                cell,
                "k must be between 0 and 1".to_string(),
            );
        }

        let n = values.len() as f64;
        let pos = k * (n - 1.0) + 1.0;
        let m = pos.floor();
        let g = pos - m;
        let idx = (m as usize).saturating_sub(1);

        if idx >= values.len() - 1 {
            return CalcResult::Number(values[values.len() - 1]);
        }

        let result = values[idx] + g * (values[idx + 1] - values[idx]);
        CalcResult::Number(result)
    }

    /// PERCENTILE.EXC(array, k)
    /// Returns the k-th percentile of values in a range, where k is in (0,1).
    pub(crate) fn fn_percentile_exc(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }

        let mut values = match self.collect_percentile_values(&args[0], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };

        if values.is_empty() {
            return CalcResult::new_error(Error::NUM, cell, "Empty array".to_string());
        }

        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

        let k = match self.get_number(&args[1], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };

        let n = values.len() as f64;

        if k <= 0.0 || k >= 1.0 {
            return CalcResult::new_error(
                Error::NUM,
                cell,
                "k must be strictly between 0 and 1".to_string(),
            );
        }

        let pos = k * (n + 1.0);
        if pos < 1.0 || pos > n {
            return CalcResult::new_error(
                Error::NUM,
                cell,
                "k out of range for data size".to_string(),
            );
        }

        let m = pos.floor();
        let g = pos - m;
        let idx = (m as usize).saturating_sub(1);

        if idx >= values.len() - 1 {
            return CalcResult::Number(values[values.len() - 1]);
        }

        let result = values[idx] + g * (values[idx + 1] - values[idx]);
        CalcResult::Number(result)
    }

    /// PERCENTRANK.INC(array, x, [significance])
    /// Returns the rank of a value as a percentage of the data set (inclusive).
    pub(crate) fn fn_percentrank_inc(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        if !(2..=3).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }

        let mut values = match self.collect_percentile_values(&args[0], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };

        if values.is_empty() {
            return CalcResult::new_error(Error::NUM, cell, "Empty array".to_string());
        }

        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

        let x = match self.get_number(&args[1], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };

        let significance = if args.len() == 3 {
            match self.get_number(&args[2], cell) {
                Ok(v) => v as i32,
                Err(e) => return e,
            }
        } else {
            3
        };

        let n = values.len() as f64;

        // Handle single element array
        if n == 1.0 {
            if (x - values[0]).abs() <= f64::EPSILON {
                // Single element exact match returns 0.5
                let factor = 10f64.powi(significance);
                let result = (0.5 * factor).floor() / factor;
                return CalcResult::Number(result);
            } else {
                return CalcResult::new_error(Error::NA, cell, "Value not found".to_string());
            }
        }

        // Handle boundary cases - clamp to 0 or 1
        if x < values[0] {
            return CalcResult::Number(0.0);
        }
        if x > values[values.len() - 1] {
            return CalcResult::Number(1.0);
        }

        // Find position
        let mut idx = 0;
        while idx < values.len() && values[idx] < x {
            idx += 1;
        }

        let rank = if idx < values.len() && (x - values[idx]).abs() <= f64::EPSILON {
            // Exact match
            idx as f64
        } else if idx == 0 {
            0.0
        } else {
            // Interpolate
            let lower = values[idx - 1];
            let upper = values[idx];
            (idx as f64 - 1.0) + (x - lower) / (upper - lower)
        };

        let mut result = rank / (n - 1.0);
        let factor = 10f64.powi(significance);
        result = (result * factor).round() / factor;
        CalcResult::Number(result)
    }

    /// PERCENTRANK.EXC(array, x, [significance])
    /// Returns the rank of a value as a percentage of the data set (exclusive).
    pub(crate) fn fn_percentrank_exc(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        if !(2..=3).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }

        let mut values = match self.collect_percentile_values(&args[0], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };

        if values.is_empty() {
            return CalcResult::new_error(Error::NUM, cell, "Empty array".to_string());
        }

        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

        let x = match self.get_number(&args[1], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };

        let significance = if args.len() == 3 {
            match self.get_number(&args[2], cell) {
                Ok(v) => v as i32,
                Err(e) => return e,
            }
        } else {
            3
        };

        let n = values.len();

        // Exclusive: x must be strictly within the range
        if x <= values[0] || x >= values[n - 1] {
            return CalcResult::new_error(Error::NUM, cell, "x out of range".to_string());
        }

        // Find position
        let mut idx = 0;
        while idx < n && values[idx] < x {
            idx += 1;
        }

        let rank = if (x - values[idx]).abs() > f64::EPSILON {
            // Interpolate
            let lower = values[idx - 1];
            let upper = values[idx];
            idx as f64 + (x - lower) / (upper - lower)
        } else {
            (idx + 1) as f64
        };

        let mut result = rank / ((n + 1) as f64);
        let factor = 10f64.powi(significance);
        result = (result * factor).round() / factor;
        CalcResult::Number(result)
    }
}
