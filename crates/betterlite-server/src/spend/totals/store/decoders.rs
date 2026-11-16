use betterlite_db::postgres_store::errors::backend;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use tokio_postgres::Row;

use super::super::rows::{
    DailySpend, KeyDailySpend, KeySpend, ModelSpend, TeamSpend, TotalSpendRow, UserKeyDailySpend,
    UserModelSpend,
};
use super::TotalsStoreError;

const SPEND_COLUMN: &str = "spend";
const SPEND_TEXT_COLUMN: &str = "spend_text";
const TEAM_SPEND_COLUMN: &str = "total_spend";

pub(super) fn daily_spend(row: &Row) -> Result<DailySpend, TotalsStoreError> {
    let date = row.try_get::<_, NaiveDate>("date").map_err(backend)?;
    let spend = row
        .try_get::<_, Option<String>>(SPEND_COLUMN)
        .map_err(backend)?;
    Ok(DailySpend {
        date,
        spend: required_amount(spend, SPEND_COLUMN)?,
    })
}

pub(super) fn key_daily_spend(row: &Row) -> Result<KeyDailySpend, TotalsStoreError> {
    let date = row.try_get::<_, NaiveDate>("date").map_err(backend)?;
    let spend = row
        .try_get::<_, Option<String>>(SPEND_COLUMN)
        .map_err(backend)?;
    let api_key = row.try_get::<_, String>("api_key").map_err(backend)?;
    Ok(KeyDailySpend {
        date,
        spend: required_amount(spend, SPEND_COLUMN)?,
        api_key,
    })
}

pub(super) fn user_daily_spend(row: &Row) -> Result<UserKeyDailySpend, TotalsStoreError> {
    let date = row.try_get::<_, NaiveDate>("date").map_err(backend)?;
    let spend = row
        .try_get::<_, Option<String>>(SPEND_COLUMN)
        .map_err(backend)?;
    let api_key = row.try_get::<_, String>("api_key").map_err(backend)?;
    let user = row.try_get::<_, String>("user").map_err(backend)?;
    Ok(UserKeyDailySpend {
        date,
        spend: required_amount(spend, SPEND_COLUMN)?,
        api_key,
        user,
    })
}

pub(super) fn total_spend(row: &Row) -> Result<TotalSpendRow, TotalsStoreError> {
    let total_spend = row
        .try_get::<_, Option<String>>(SPEND_TEXT_COLUMN)
        .map_err(backend)?;
    Ok(TotalSpendRow {
        total_spend: parse_amount(total_spend, SPEND_TEXT_COLUMN)?,
    })
}

pub(super) fn key_spend(row: &Row) -> Result<KeySpend, TotalsStoreError> {
    let api_key = row.try_get::<_, String>("api_key").map_err(backend)?;
    let total_spend = row
        .try_get::<_, Option<String>>(SPEND_TEXT_COLUMN)
        .map_err(backend)?;
    let key_alias = row
        .try_get::<_, Option<String>>("key_alias")
        .map_err(backend)?;
    let key_name = row
        .try_get::<_, Option<String>>("key_name")
        .map_err(backend)?;
    Ok(KeySpend {
        api_key,
        total_spend: required_amount(total_spend, SPEND_TEXT_COLUMN)?,
        key_alias,
        key_name,
    })
}

pub(super) fn model_spend(row: &Row) -> Result<ModelSpend, TotalsStoreError> {
    let model = row.try_get::<_, String>("model").map_err(backend)?;
    let total_spend = row
        .try_get::<_, Option<String>>(SPEND_TEXT_COLUMN)
        .map_err(backend)?;
    Ok(ModelSpend {
        model,
        total_spend: required_amount(total_spend, SPEND_TEXT_COLUMN)?,
    })
}

pub(super) fn user_model_spend(row: &Row) -> Result<UserModelSpend, TotalsStoreError> {
    let model = row.try_get::<_, String>("model").map_err(backend)?;
    let total_spend = row
        .try_get::<_, Option<String>>(SPEND_TEXT_COLUMN)
        .map_err(backend)?;
    let total_tokens = row.try_get::<_, i64>("total_tokens").map_err(backend)?;
    Ok(UserModelSpend {
        model,
        total_spend: required_amount(total_spend, SPEND_TEXT_COLUMN)?,
        total_tokens,
    })
}

pub(super) fn team_spend(row: &Row) -> Result<TeamSpend, TotalsStoreError> {
    let team_alias = row
        .try_get::<_, Option<String>>("team_alias")
        .map_err(backend)?;
    let spend_date = row
        .try_get::<_, Option<NaiveDate>>("spend_date")
        .map_err(backend)?;
    let total_spend = row
        .try_get::<_, Option<f64>>(TEAM_SPEND_COLUMN)
        .map_err(backend)?;
    Ok(TeamSpend {
        team_alias,
        spend_date,
        total_spend: retained_amount(total_spend, TEAM_SPEND_COLUMN)?,
    })
}

fn required_amount(
    text: Option<String>,
    column: &'static str,
) -> Result<Decimal, TotalsStoreError> {
    match parse_amount(text, column)? {
        Some(value) => Ok(value),
        None => Err(TotalsStoreError::InvalidAmount { column }),
    }
}

fn parse_amount(
    text: Option<String>,
    column: &'static str,
) -> Result<Option<Decimal>, TotalsStoreError> {
    let Some(text) = text else {
        return Ok(None);
    };
    match text.parse::<Decimal>() {
        Ok(value) => Ok(Some(value)),
        Err(_source) => Decimal::from_scientific(&text)
            .map(Some)
            .map_err(|_source| TotalsStoreError::InvalidAmount { column }),
    }
}

fn retained_amount(
    value: Option<f64>,
    column: &'static str,
) -> Result<Option<Decimal>, TotalsStoreError> {
    let Some(value) = value else {
        return Ok(None);
    };
    Decimal::from_f64_retain(value)
        .map(Some)
        .ok_or(TotalsStoreError::InvalidAmount { column })
}
