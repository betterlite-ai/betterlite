use std::str::FromStr;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::finops::cloudzero::UsageRow;
use betterlite_logging::finops::cloudzero::records::UsageRecord;
use rust_decimal::Decimal;
use thiserror::Error;

// A usage row arrives as JSON columns; a column of another type is refused by name.
#[derive(Debug, Error)]
pub enum UsageRowError {
    #[error("the usage row field '{field}' is not text")]
    NotText { field: &'static str },
    #[error("the usage row field '{field}' is not an integer")]
    NotInteger { field: &'static str },
    #[error("the usage row field '{field}' is not a decimal number")]
    NotDecimal { field: &'static str },
    #[error("the usage row field '{field}' is not a valid decimal number")]
    Decimal {
        field: &'static str,
        #[source]
        source: rust_decimal::Error,
    },
}

pub fn usage_records(rows: &[UsageRow]) -> Result<Vec<UsageRecord>, UsageRowError> {
    rows.iter().map(usage_record).collect()
}

fn usage_record(row: &UsageRow) -> Result<UsageRecord, UsageRowError> {
    Ok(UsageRecord {
        date: text_field(row, "date")?.map(str::to_owned),
        custom_llm_provider: text_field(row, "custom_llm_provider")?
            .unwrap_or_default()
            .to_owned(),
        team_id: text_field(row, "team_id")?.unwrap_or_default().to_owned(),
        team_alias: text_field(row, "team_alias")?.map(str::to_owned),
        model: text_field(row, "model")?.unwrap_or_default().to_owned(),
        model_group: text_field(row, "model_group")?
            .unwrap_or_default()
            .to_owned(),
        api_key: text_field(row, "api_key")?.unwrap_or_default().to_owned(),
        api_key_alias: text_field(row, "api_key_alias")?.map(str::to_owned),
        user_email: text_field(row, "user_email")?.map(str::to_owned),
        organization_alias: text_field(row, "organization_alias")?.map(str::to_owned),
        project_alias: text_field(row, "project_alias")?.map(str::to_owned),
        user_alias: text_field(row, "user_alias")?.map(str::to_owned),
        api_requests: integer_field(row, "api_requests")?,
        successful_requests: integer_field(row, "successful_requests")?,
        failed_requests: integer_field(row, "failed_requests")?,
        prompt_tokens: integer_field(row, "prompt_tokens")?,
        completion_tokens: integer_field(row, "completion_tokens")?,
        cache_creation_input_tokens: integer_field(row, "cache_creation_input_tokens")?,
        cache_read_input_tokens: integer_field(row, "cache_read_input_tokens")?,
        spend: decimal_field(row, "spend")?,
    })
}

pub(crate) fn text_field<'row>(
    row: &'row UsageRow,
    field: &'static str,
) -> Result<Option<&'row str>, UsageRowError> {
    match row.get(field) {
        None | Some(JsonData::Null) => Ok(None),
        Some(JsonData::String(text)) => Ok(Some(text.as_str())),
        Some(_) => Err(UsageRowError::NotText { field }),
    }
}

pub(crate) fn integer_field(row: &UsageRow, field: &'static str) -> Result<i64, UsageRowError> {
    match row.get(field) {
        None | Some(JsonData::Null) => Ok(0),
        Some(JsonData::Number(number)) => {
            number.as_i64().ok_or(UsageRowError::NotInteger { field })
        }
        Some(_) => Err(UsageRowError::NotInteger { field }),
    }
}

pub(crate) fn decimal_field(row: &UsageRow, field: &'static str) -> Result<Decimal, UsageRowError> {
    match row.get(field) {
        None | Some(JsonData::Null) => Ok(Decimal::ZERO),
        Some(JsonData::Number(number)) => Decimal::from_str(number.as_str())
            .map_err(|source| UsageRowError::Decimal { field, source }),
        Some(_) => Err(UsageRowError::NotDecimal { field }),
    }
}
