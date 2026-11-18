use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::daily_spend_sql::entities::SpendRow;
use crate::daily_spend_sql::values::SpendColumnValue;
use crate::spend_queues::SpendQueueError;

pub(super) fn add_counter(
    existing: &mut SpendRow,
    incoming: &SpendRow,
    field: &'static str,
    required: bool,
) -> Result<(), SpendQueueError> {
    let current = match existing.get(field) {
        Some(value) => counter_value(value, field)?,
        None if required => return Err(SpendQueueError::MissingField { field }),
        None => 0,
    };
    // The source indexes a required field and coalesces only an optional one, so a null required value stays an invalid addition.
    let added = match incoming.get(field) {
        Some(value) if required => counter_value(value, field)?,
        Some(value) => incoming_counter(value, field)?,
        None if required => return Err(SpendQueueError::MissingField { field }),
        None => 0,
    };
    let total = current
        .checked_add(added)
        .ok_or(SpendQueueError::FieldOverflow { field })?;
    existing.insert(field.to_owned(), SpendColumnValue::Int(total));
    Ok(())
}

pub(super) fn add_money(
    existing: &mut SpendRow,
    incoming: &SpendRow,
    field: &'static str,
    required: bool,
) -> Result<(), SpendQueueError> {
    let current = match existing.get(field) {
        Some(value) => money_value(value, field)?,
        None if required => return Err(SpendQueueError::MissingField { field }),
        None => Money::ZERO,
    };
    let added = match incoming.get(field) {
        Some(value) if required => money_value(value, field)?,
        Some(value) => incoming_money(value, field)?,
        None if required => return Err(SpendQueueError::MissingField { field }),
        None => Money::ZERO,
    };
    let total = current
        .amount
        .checked_add(added.amount)
        .ok_or(SpendQueueError::FieldOverflow { field })?;
    // An integer sum of integer inputs stays an integer, as Python's int + int does.
    let stored = if current.fractional || added.fractional {
        SpendColumnValue::Decimal(total)
    } else {
        total
            .to_i64()
            .map_or(SpendColumnValue::Decimal(total), SpendColumnValue::Int)
    };
    existing.insert(field.to_owned(), stored);
    Ok(())
}

fn incoming_counter(value: &SpendColumnValue, field: &'static str) -> Result<i64, SpendQueueError> {
    if is_falsey(value) {
        return Ok(0);
    }
    counter_value(value, field)
}

fn incoming_money(value: &SpendColumnValue, field: &'static str) -> Result<Money, SpendQueueError> {
    if is_falsey(value) {
        return Ok(Money::ZERO);
    }
    money_value(value, field)
}

fn counter_value(value: &SpendColumnValue, field: &'static str) -> Result<i64, SpendQueueError> {
    match value {
        SpendColumnValue::Bool(flag) => Ok(i64::from(*flag)),
        SpendColumnValue::Int(whole) => Ok(*whole),
        SpendColumnValue::Decimal(amount) => amount
            .trunc()
            .to_i64()
            .ok_or(SpendQueueError::FieldOverflow { field }),
        SpendColumnValue::Null
        | SpendColumnValue::Text(_)
        | SpendColumnValue::Array(_)
        | SpendColumnValue::Object(_) => Err(SpendQueueError::InvalidAddition { field }),
    }
}

fn money_value(value: &SpendColumnValue, field: &'static str) -> Result<Money, SpendQueueError> {
    match value {
        SpendColumnValue::Bool(flag) => Ok(Money {
            amount: Decimal::from(u8::from(*flag)),
            fractional: false,
        }),
        SpendColumnValue::Int(whole) => Ok(Money {
            amount: Decimal::from(*whole),
            fractional: false,
        }),
        SpendColumnValue::Decimal(amount) => Ok(Money {
            amount: *amount,
            fractional: true,
        }),
        SpendColumnValue::Null
        | SpendColumnValue::Text(_)
        | SpendColumnValue::Array(_)
        | SpendColumnValue::Object(_) => Err(SpendQueueError::InvalidAddition { field }),
    }
}

// Python truthiness of the value the source writes `... or 0` around.
fn is_falsey(value: &SpendColumnValue) -> bool {
    match value {
        SpendColumnValue::Null => true,
        SpendColumnValue::Bool(flag) => !*flag,
        SpendColumnValue::Int(whole) => *whole == 0,
        SpendColumnValue::Decimal(amount) => *amount == Decimal::ZERO,
        SpendColumnValue::Text(text) => text.is_empty(),
        SpendColumnValue::Array(values) => values.is_empty(),
        SpendColumnValue::Object(fields) => fields.is_empty(),
    }
}

#[derive(Debug, Clone, Copy)]
struct Money {
    amount: Decimal,
    fractional: bool,
}

impl Money {
    const ZERO: Self = Self {
        amount: Decimal::ZERO,
        fractional: false,
    };
}
