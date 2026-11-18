use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenSqlType;

use super::KeyUserWriteError;
use super::columns::{UserColumn, UserColumnDef, UserColumnOp, UserUpdateOps};
use super::sql::{Parameter, Statement, parameter_for};

pub(super) fn assignments(columns: &[String], placeholders: &[String]) -> Vec<String> {
    let mut sets = Vec::with_capacity(columns.len());
    for (column, marker) in columns.iter().zip(placeholders) {
        sets.push(format!("{column} = {marker}"));
    }
    sets
}

pub(super) fn update_assignments(
    statement: &mut Statement,
    update: &UserUpdateOps,
) -> Result<Vec<String>, KeyUserWriteError> {
    let mut sets = Vec::with_capacity(update.0.len());
    for (column, op) in &update.0 {
        sets.push(assignment(statement, *column, op)?);
    }
    Ok(sets)
}

fn assignment(
    statement: &mut Statement,
    column: UserColumn,
    op: &UserColumnOp,
) -> Result<String, KeyUserWriteError> {
    let definition = column.definition();
    let (operator, value) = match op {
        UserColumnOp::Set(value) => {
            let parameter = parameter_for(definition.sql_type, value)?;
            let index = statement.parameters.len().saturating_add(1);
            let marker = parameter.marker(index, definition.sql_type);
            statement.parameters.push(parameter);
            return Ok(format!("\"{}\" = {marker}", definition.name));
        }
        UserColumnOp::Increment(value) => ("+", value),
        UserColumnOp::Decrement(value) => ("-", value),
        UserColumnOp::Multiply(value) => ("*", value),
        UserColumnOp::Divide(value) => ("/", value),
    };
    let (cast, operand) = numeric_operand(definition, value)?;
    let index = statement.parameters.len().saturating_add(1);
    statement.parameters.push(Parameter::Text(Some(operand)));
    Ok(format!(
        "\"{}\" = r.\"{}\" {operator} ${index}::text::{cast}",
        definition.name, definition.name
    ))
}

fn numeric_operand(
    definition: UserColumnDef,
    value: &TokenValue,
) -> Result<(&'static str, String), KeyUserWriteError> {
    let cast = match definition.sql_type {
        TokenSqlType::DoublePrecision => "double precision",
        TokenSqlType::BigInt => "bigint",
        TokenSqlType::Text
        | TokenSqlType::Boolean
        | TokenSqlType::Json
        | TokenSqlType::Timestamp
        | TokenSqlType::TextArray => {
            return Err(KeyUserWriteError::NonNumericUpdate {
                column: definition.name,
            });
        }
    };
    let operand = match value {
        TokenValue::Integer(number) => number.to_string(),
        TokenValue::Number(number) => number.to_string(),
        TokenValue::Text(_)
        | TokenValue::KeyHash(_)
        | TokenValue::Bool(_)
        | TokenValue::Json(_)
        | TokenValue::Timestamp(_)
        | TokenValue::TextArray(_)
        | TokenValue::Null => {
            return Err(KeyUserWriteError::NonNumericUpdate {
                column: definition.name,
            });
        }
    };
    Ok((cast, operand))
}
