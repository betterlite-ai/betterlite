use betterlite_db::postgres_store::errors::backend;
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

impl TryFrom<&Row> for DailySpend {
    type Error = TotalsStoreError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            date: row.try_get("date").map_err(backend)?,
            spend: required_amount(row, SPEND_COLUMN)?,
        })
    }
}

impl TryFrom<&Row> for KeyDailySpend {
    type Error = TotalsStoreError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            date: row.try_get("date").map_err(backend)?,
            spend: required_amount(row, SPEND_COLUMN)?,
            api_key: row.try_get("api_key").map_err(backend)?,
        })
    }
}

impl TryFrom<&Row> for UserKeyDailySpend {
    type Error = TotalsStoreError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        let user = row.try_get("user").map_err(backend)?;
        Ok(Self::from((KeyDailySpend::try_from(row)?, user)))
    }
}

impl TryFrom<&Row> for TotalSpendRow {
    type Error = TotalsStoreError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            total_spend: optional_amount(row, SPEND_TEXT_COLUMN)?,
        })
    }
}

impl TryFrom<&Row> for KeySpend {
    type Error = TotalsStoreError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        let api_key = row.try_get("api_key").map_err(backend)?;
        let key_alias = row.try_get("key_alias").map_err(backend)?;
        let key_name = row.try_get("key_name").map_err(backend)?;
        Ok(Self {
            api_key,
            total_spend: required_amount(row, SPEND_TEXT_COLUMN)?,
            key_alias,
            key_name,
        })
    }
}

impl TryFrom<&Row> for ModelSpend {
    type Error = TotalsStoreError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            model: row.try_get("model").map_err(backend)?,
            total_spend: required_amount(row, SPEND_TEXT_COLUMN)?,
        })
    }
}

impl TryFrom<&Row> for UserModelSpend {
    type Error = TotalsStoreError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            model: row.try_get("model").map_err(backend)?,
            total_spend: required_amount(row, SPEND_TEXT_COLUMN)?,
            total_tokens: row.try_get("total_tokens").map_err(backend)?,
        })
    }
}

impl TryFrom<&Row> for TeamSpend {
    type Error = TotalsStoreError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            team_alias: row.try_get("team_alias").map_err(backend)?,
            spend_date: row.try_get("spend_date").map_err(backend)?,
            total_spend: retained_amount(row, TEAM_SPEND_COLUMN)?,
        })
    }
}

fn required_amount(row: &Row, column: &'static str) -> Result<Decimal, TotalsStoreError> {
    let Some(amount) = optional_amount(row, column)? else {
        return Err(TotalsStoreError::InvalidAmount { column });
    };
    Ok(amount)
}

fn optional_amount(row: &Row, column: &'static str) -> Result<Option<Decimal>, TotalsStoreError> {
    let stored = row.try_get::<_, Option<String>>(column).map_err(backend)?;
    stored
        .map(|digits| parse_amount(&digits, column))
        .transpose()
}

fn parse_amount(digits: &str, column: &'static str) -> Result<Decimal, TotalsStoreError> {
    digits
        .parse::<Decimal>()
        .or_else(|_unparsed| Decimal::from_scientific(digits))
        .map_err(|source| TotalsStoreError::UnreadableAmount { column, source })
}

fn retained_amount(row: &Row, column: &'static str) -> Result<Option<Decimal>, TotalsStoreError> {
    let stored = row.try_get::<_, Option<f64>>(column).map_err(backend)?;
    let Some(float) = stored else {
        return Ok(None);
    };
    Decimal::from_f64_retain(float)
        .map(Some)
        .ok_or(TotalsStoreError::InvalidAmount { column })
}
