//! In-place displacement of a formula's references after a structural edit.
//!
//! Structural edits (insert/delete rows/columns, row/column moves) shift the
//! references in every formula. The reference-shifting rule is the numeric core
//! in [`super::stringify::displace_resolved_coordinates`], shared with the
//! stringifier so the two can never diverge.
//!
//! [`displace_node`] rewrites the parsed AST directly, avoiding the
//! stringify-then-reparse round trip the string path pays per formula. It is
//! deliberately conservative: any case it does not model in place (a reference
//! displaced off the sheet, i.e. `#REF!`, or a malformed reference) returns
//! `None`, and the caller falls back to the exact string path. So the fast path
//! only ever produces a result the string path would also produce.

use super::stringify::{displace_resolved_coordinates, DisplaceData};
use super::Node;
use crate::constants::{LAST_COLUMN, LAST_ROW};
use crate::number_format::to_excel_precision;

/// Shifts a single reference resolved against the owning cell at
/// `(context_row, context_column)`. Returns `None` for `#REF!`.
#[allow(clippy::too_many_arguments)]
fn displace_reference(
    sheet_index: u32,
    absolute_row: bool,
    absolute_column: bool,
    row: i32,
    column: i32,
    context_row: i32,
    context_column: i32,
    full_row: bool,
    full_column: bool,
    displace_data: &DisplaceData,
) -> Option<(i32, i32)> {
    let resolved_row = if absolute_row { row } else { row + context_row };
    let resolved_column = if absolute_column {
        column
    } else {
        column + context_column
    };
    let (new_row, new_column) = displace_resolved_coordinates(
        sheet_index,
        resolved_row,
        resolved_column,
        full_row,
        full_column,
        displace_data,
    )?;
    let stored_row = if absolute_row {
        new_row
    } else {
        new_row - context_row
    };
    let stored_column = if absolute_column {
        new_column
    } else {
        new_column - context_column
    };
    Some((stored_row, stored_column))
}

/// Returns the formula with its references displaced, or `None` when any case is
/// not modeled in place and the caller should fall back to the string path. The
/// context is the owning cell's absolute position, which relative references
/// resolve against.
pub(crate) fn displace_node(
    node: &Node,
    context_row: i32,
    context_column: i32,
    displace_data: &DisplaceData,
) -> Option<Node> {
    match node {
        Node::BooleanKind(_)
        | Node::StringKind(_)
        | Node::DefinedNameKind(_)
        | Node::TableNameKind(_)
        | Node::NamedVariableKind { .. }
        | Node::ErrorKind(_)
        | Node::EmptyArgKind => Some(node.clone()),
        Node::NumberKind(n) => Some(Node::NumberKind(to_excel_precision(*n, 15))),
        Node::ArrayKind(array) => Some(Node::ArrayKind(
            array
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|value| match value {
                            super::ArrayNode::Number(n) => {
                                super::ArrayNode::Number(to_excel_precision(*n, 15))
                            }
                            other => other.clone(),
                        })
                        .collect()
                })
                .collect(),
        )),
        Node::ReferenceKind {
            sheet_name,
            sheet_index,
            absolute_row,
            absolute_column,
            row,
            column,
        } => {
            let (row, column) = displace_reference(
                *sheet_index,
                *absolute_row,
                *absolute_column,
                *row,
                *column,
                context_row,
                context_column,
                false, // full_row: a single cell never spans a whole row
                false, // full_column
                displace_data,
            )?;
            Some(Node::ReferenceKind {
                sheet_name: sheet_name.clone(),
                sheet_index: *sheet_index,
                absolute_row: *absolute_row,
                absolute_column: *absolute_column,
                row,
                column,
            })
        }
        Node::RangeKind {
            sheet_name,
            sheet_index,
            absolute_row1,
            absolute_column1,
            row1,
            column1,
            absolute_row2,
            absolute_column2,
            row2,
            column2,
        } => {
            // Open ranges (A:A, 1:1) keep their spanning axis fixed, matching the
            // stringifier's full_row/full_column handling.
            let full_row = *absolute_row1 && *absolute_row2 && *row1 == 1 && *row2 == LAST_ROW;
            let full_column =
                *absolute_column1 && *absolute_column2 && *column1 == 1 && *column2 == LAST_COLUMN;
            let (mut row1, mut column1) = displace_reference(
                *sheet_index,
                *absolute_row1,
                *absolute_column1,
                *row1,
                *column1,
                context_row,
                context_column,
                full_row,
                full_column,
                displace_data,
            )?;
            let (mut row2, mut column2) = displace_reference(
                *sheet_index,
                *absolute_row2,
                *absolute_column2,
                *row2,
                *column2,
                context_row,
                context_column,
                full_row,
                full_column,
                displace_data,
            )?;
            let mut absolute_row1 = *absolute_row1;
            let mut absolute_row2 = *absolute_row2;
            let mut absolute_column1 = *absolute_column1;
            let mut absolute_column2 = *absolute_column2;
            // A displacement can cross the two endpoints of a range (e.g. a move
            // that shifts them by different amounts). Reparsing an A1 range
            // normalizes that by swapping the reversed endpoints together with
            // their absolute flags; mirror it so the AST matches the string path.
            let resolved_row1 = if absolute_row1 {
                row1
            } else {
                row1 + context_row
            };
            let resolved_row2 = if absolute_row2 {
                row2
            } else {
                row2 + context_row
            };
            if resolved_row1 > resolved_row2 {
                (row1, row2) = (row2, row1);
                (absolute_row1, absolute_row2) = (absolute_row2, absolute_row1);
            }
            let resolved_column1 = if absolute_column1 {
                column1
            } else {
                column1 + context_column
            };
            let resolved_column2 = if absolute_column2 {
                column2
            } else {
                column2 + context_column
            };
            if resolved_column1 > resolved_column2 {
                (column1, column2) = (column2, column1);
                (absolute_column1, absolute_column2) = (absolute_column2, absolute_column1);
            }
            Some(Node::RangeKind {
                sheet_name: sheet_name.clone(),
                sheet_index: *sheet_index,
                absolute_row1,
                absolute_column1,
                row1,
                column1,
                absolute_row2,
                absolute_column2,
                row2,
                column2,
            })
        }
        Node::OpRangeKind { left, right } => Some(Node::OpRangeKind {
            left: Box::new(displace_node(
                left,
                context_row,
                context_column,
                displace_data,
            )?),
            right: Box::new(displace_node(
                right,
                context_row,
                context_column,
                displace_data,
            )?),
        }),
        Node::OpConcatenateKind { left, right } => Some(Node::OpConcatenateKind {
            left: Box::new(displace_node(
                left,
                context_row,
                context_column,
                displace_data,
            )?),
            right: Box::new(displace_node(
                right,
                context_row,
                context_column,
                displace_data,
            )?),
        }),
        Node::OpSumKind { kind, left, right } => Some(Node::OpSumKind {
            kind: kind.clone(),
            left: Box::new(displace_node(
                left,
                context_row,
                context_column,
                displace_data,
            )?),
            right: Box::new(displace_node(
                right,
                context_row,
                context_column,
                displace_data,
            )?),
        }),
        Node::OpProductKind { kind, left, right } => Some(Node::OpProductKind {
            kind: kind.clone(),
            left: Box::new(displace_node(
                left,
                context_row,
                context_column,
                displace_data,
            )?),
            right: Box::new(displace_node(
                right,
                context_row,
                context_column,
                displace_data,
            )?),
        }),
        Node::OpPowerKind { left, right } => Some(Node::OpPowerKind {
            left: Box::new(displace_node(
                left,
                context_row,
                context_column,
                displace_data,
            )?),
            right: Box::new(displace_node(
                right,
                context_row,
                context_column,
                displace_data,
            )?),
        }),
        Node::CompareKind { kind, left, right } => Some(Node::CompareKind {
            kind: kind.clone(),
            left: Box::new(displace_node(
                left,
                context_row,
                context_column,
                displace_data,
            )?),
            right: Box::new(displace_node(
                right,
                context_row,
                context_column,
                displace_data,
            )?),
        }),
        Node::UnaryKind { kind, right } => Some(Node::UnaryKind {
            kind: kind.clone(),
            right: Box::new(displace_node(
                right,
                context_row,
                context_column,
                displace_data,
            )?),
        }),
        Node::ImplicitIntersection { automatic, child } => Some(Node::ImplicitIntersection {
            automatic: *automatic,
            child: Box::new(displace_node(
                child,
                context_row,
                context_column,
                displace_data,
            )?),
        }),
        Node::SpillRangeOperator { child } => Some(Node::SpillRangeOperator {
            child: Box::new(displace_node(
                child,
                context_row,
                context_column,
                displace_data,
            )?),
        }),
        Node::FunctionKind { kind, args } => Some(Node::FunctionKind {
            kind: kind.clone(),
            args: displace_args(args, context_row, context_column, displace_data)?,
        }),
        Node::NamedFunctionKind { id, name, args } => Some(Node::NamedFunctionKind {
            id: *id,
            name: name.clone(),
            args: displace_args(args, context_row, context_column, displace_data)?,
        }),
        Node::LambdaCallKind { lambda, args } => Some(Node::LambdaCallKind {
            lambda: Box::new(displace_node(
                lambda,
                context_row,
                context_column,
                displace_data,
            )?),
            args: displace_args(args, context_row, context_column, displace_data)?,
        }),
        Node::LambdaDefKind { parameters, body } => Some(Node::LambdaDefKind {
            parameters: parameters.clone(),
            body: Box::new(displace_node(
                body,
                context_row,
                context_column,
                displace_data,
            )?),
        }),
        // Malformed references and parse errors are left to the string path,
        // which mirrors Excel's handling exactly.
        Node::WrongReferenceKind { .. }
        | Node::WrongRangeKind { .. }
        | Node::ParseErrorKind { .. } => None,
    }
}

fn displace_args(
    args: &[Node],
    context_row: i32,
    context_column: i32,
    displace_data: &DisplaceData,
) -> Option<Vec<Node>> {
    args.iter()
        .map(|arg| displace_node(arg, context_row, context_column, displace_data))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::expressions::parser::new_parser_english;
    use crate::expressions::parser::stringify::{to_rc_format, to_string_displaced};
    use crate::expressions::types::CellReferenceRC;
    use crate::language::get_default_language;
    use crate::locale::get_default_locale;

    // The fast path must return exactly what the string path would: reparse the
    // displaced rendering and compare the canonical R1C1 form. When
    // `displace_node` declines (returns None), the caller uses the string path,
    // so there is nothing to compare.
    fn assert_matches_string_path(formula: &str, row: i32, column: i32, displace: &DisplaceData) {
        let mut parser = new_parser_english(vec!["Sheet1".to_string()], vec![], HashMap::new());
        let context = CellReferenceRC {
            sheet: "Sheet1".to_string(),
            row,
            column,
        };
        let node = parser.parse(formula, &context);
        let Some(fast) = displace_node(&node, row, column, displace) else {
            return;
        };
        let displaced = to_string_displaced(
            &node,
            &context,
            displace,
            get_default_locale(),
            get_default_language(),
        );
        let reparsed = parser.parse(&displaced, &context);
        assert_eq!(
            to_rc_format(&fast),
            to_rc_format(&reparsed),
            "formula {formula} at ({row},{column}) under {displace:?}"
        );
    }

    #[test]
    fn matches_string_path_across_formulas_and_displacements() {
        let formulas = [
            "A1+1",
            "$A$1+B2",
            "$A1+A$1",
            "SUM(A1:B10)",
            "SUM(A:A)",
            "SUM(1:1)",
            "A5*C3-D$2",
            "SUM(B2:B100)+MAX(C1:C50)",
            "(A1+A2)*B3^2",
            "-A10%",
            // Ranges whose two endpoints carry different absolute flags: a move
            // that crosses them must swap the flags with the coordinates, so
            // these exercise the endpoint-normalization branch.
            "SUM($A$2:A10)",
            "SUM(A$3:C$8)",
            "SUM($B5:D8)",
            "SUM(A2:$A$10)",
        ];
        let displacements = [
            DisplaceData::Row {
                sheet: 0,
                row: 3,
                delta: 2,
            },
            DisplaceData::Row {
                sheet: 0,
                row: 3,
                delta: -2,
            },
            DisplaceData::Row {
                sheet: 0,
                row: 1,
                delta: 5,
            },
            DisplaceData::Column {
                sheet: 0,
                column: 2,
                delta: 3,
            },
            DisplaceData::Column {
                sheet: 0,
                column: 2,
                delta: -1,
            },
            // A different sheet must leave every reference untouched.
            DisplaceData::Row {
                sheet: 1,
                row: 1,
                delta: 4,
            },
            // Moves can shift the two endpoints of a range by different amounts
            // and cross them, exercising endpoint normalization.
            // Boundary: moving row 1 / column 1 used to shift A:A / 1:1
            // endpoints. The string path hid that by omitting the axis at
            // render time; the AST path persisted it.
            DisplaceData::RowMove {
                sheet: 0,
                row: 1,
                delta: 2,
            },
            DisplaceData::ColumnMove {
                sheet: 0,
                column: 1,
                delta: 2,
            },
            DisplaceData::RowMove {
                sheet: 0,
                row: 3,
                delta: 4,
            },
            DisplaceData::RowMove {
                sheet: 0,
                row: 10,
                delta: -6,
            },
            DisplaceData::ColumnMove {
                sheet: 0,
                column: 2,
                delta: 3,
            },
            DisplaceData::CellHorizontal {
                sheet: 0,
                row: 5,
                column: 2,
                delta: 2,
            },
            DisplaceData::CellVertical {
                sheet: 0,
                row: 5,
                column: 2,
                delta: 2,
            },
        ];
        // A spread of owning-cell positions so relative offsets resolve differently.
        for &(row, column) in &[(1, 1), (5, 4), (20, 10), (100, 3)] {
            for formula in &formulas {
                for displace in &displacements {
                    assert_matches_string_path(formula, row, column, displace);
                }
            }
        }
    }
}
