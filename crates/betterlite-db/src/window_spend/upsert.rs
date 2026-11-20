use chrono::NaiveDateTime;
use rust_decimal::Decimal;

use crate::window_queue::WindowSpendTransaction;

use super::error::WindowSpendError;
use super::seed::window_instant;

pub type PrimaryKey = (String, String, String);

pub struct WindowUpsertParams {
    pub entity_type: String,
    pub entity_id: String,
    pub window_duration: String,
    pub window_start: NaiveDateTime,
    pub spend: Decimal,
    pub increment: Decimal,
    pub now: NaiveDateTime,
}

#[must_use]
pub fn primary_key(transaction: &WindowSpendTransaction) -> PrimaryKey {
    (
        transaction.entity_type.clone(),
        transaction.entity_id.clone(),
        transaction.window_duration.clone(),
    )
}

pub fn upsert_params(
    transaction: &WindowSpendTransaction,
    seed_base: Decimal,
    now: NaiveDateTime,
) -> Result<WindowUpsertParams, WindowSpendError> {
    let increment = transaction.spend.0;
    Ok(WindowUpsertParams {
        entity_type: transaction.entity_type.clone(),
        entity_id: transaction.entity_id.clone(),
        window_duration: transaction.window_duration.clone(),
        window_start: window_instant(&transaction.window_start)?,
        spend: seed_base
            .checked_add(increment)
            .ok_or(WindowSpendError::Overflow)?,
        increment,
        now,
    })
}
