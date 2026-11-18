use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::daily_spend_sql::entities::SpendRow;
use crate::daily_spend_sql::values::SpendColumnValue;
use crate::spend_queues::SpendQueueError;

#[derive(Debug, Clone, Copy)]
pub(super) enum Measure {
    Counter,
    Money,
}

pub(super) fn add_metric(
    existing: &mut SpendRow,
    incoming: &SpendRow,
    field: &'static str,
    measure: Measure,
    required: bool,
) -> Result<(), SpendQueueError> {
    let current = match existing.get(field) {
        Some(value) => read_metric(value, field, measure)?,
        None if required => return Err(SpendQueueError::MissingField { field }),
        None => Amount::ZERO,
    };
    // The source indexes a required field and coalesces only an optional one, so an optional null becomes zero and a required null stays invalid.
    let added = match incoming.get(field) {
        Some(value) if required => read_metric(value, field, measure)?,
        Some(value) if is_falsey(value) => Amount::ZERO,
        Some(value) => read_metric(value, field, measure)?,
        None if required => return Err(SpendQueueError::MissingField { field }),
        None => Amount::ZERO,
    };
    let total = current
        .amount
        .checked_add(added.amount)
        .ok_or(SpendQueueError::FieldOverflow { field })?;
    existing.insert(
        field.to_owned(),
        store(
            total,
            current.fractional || added.fractional,
            measure,
            field,
        )?,
    );
    Ok(())
}

fn read_metric(
    value: &SpendColumnValue,
    field: &'static str,
    measure: Measure,
) -> Result<Amount, SpendQueueError> {
    match value {
        SpendColumnValue::Bool(flag) => Ok(Amount {
            amount: Decimal::from(u8::from(*flag)),
            fractional: false,
        }),
        SpendColumnValue::Int(whole) => Ok(Amount {
            amount: Decimal::from(*whole),
            fractional: false,
        }),
        SpendColumnValue::Decimal(amount) => match measure {
            Measure::Counter => amount
                .trunc()
                .to_i64()
                .map(|whole| Amount {
                    amount: Decimal::from(whole),
                    fractional: false,
                })
                .ok_or(SpendQueueError::FieldOverflow { field }),
            Measure::Money => Ok(Amount {
                amount: *amount,
                fractional: true,
            }),
        },
        SpendColumnValue::Null
        | SpendColumnValue::Text(_)
        | SpendColumnValue::Array(_)
        | SpendColumnValue::Object(_) => Err(SpendQueueError::InvalidAddition { field }),
    }
}

fn store(
    total: Decimal,
    fractional: bool,
    measure: Measure,
    field: &'static str,
) -> Result<SpendColumnValue, SpendQueueError> {
    if fractional {
        return Ok(SpendColumnValue::Decimal(total));
    }
    match measure {
        Measure::Counter => total
            .to_i64()
            .map(SpendColumnValue::Int)
            .ok_or(SpendQueueError::FieldOverflow { field }),
        // An integer sum of integer inputs stays an integer, as Python's int + int does.
        Measure::Money => Ok(total
            .to_i64()
            .map_or(SpendColumnValue::Decimal(total), SpendColumnValue::Int)),
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
struct Amount {
    amount: Decimal,
    fractional: bool,
}

impl Amount {
    const ZERO: Self = Self {
        amount: Decimal::ZERO,
        fractional: false,
    };
}
