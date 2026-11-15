use std::collections::BTreeMap;
use std::str::FromStr;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::finops::focus::UsageData;
use rust_decimal::Decimal;

use crate::focus_records::transformer::UsageRow;

#[derive(Debug, thiserror::Error)]
pub enum UsageRowError {
    #[error("usage row {index} has no '{field}' value")]
    Missing { index: usize, field: &'static str },
    #[error("usage row {index} has a '{field}' value of the wrong type")]
    WrongType { index: usize, field: &'static str },
    #[error("usage row {index} has a '{field}' value that is not a decimal")]
    NotDecimal {
        index: usize,
        field: &'static str,
        #[source]
        source: rust_decimal::Error,
    },
}

pub fn decode(rows: &UsageData) -> Result<Vec<UsageRow>, UsageRowError> {
    rows.iter()
        .enumerate()
        .map(|(index, row)| decode_row(index, row))
        .collect()
}

fn decode_row(index: usize, row: &BTreeMap<String, JsonData>) -> Result<UsageRow, UsageRowError> {
    Ok(UsageRow {
        date: text(index, row, "date")?.to_owned(),
        api_key: text(index, row, "api_key")?.to_owned(),
        spend: decimal(index, row, "spend")?,
        api_requests: integer(index, row, "api_requests")?,
        user_id: optional_text(index, row, "user_id")?.map(str::to_owned),
        model: optional_text(index, row, "model")?.map(str::to_owned),
        model_group: optional_text(index, row, "model_group")?.map(str::to_owned),
        custom_llm_provider: optional_text(index, row, "custom_llm_provider")?.map(str::to_owned),
        team_id: optional_text(index, row, "team_id")?.map(str::to_owned),
        team_alias: optional_text(index, row, "team_alias")?.map(str::to_owned),
        organization_id: optional_text(index, row, "organization_id")?.map(str::to_owned),
        organization_alias: optional_text(index, row, "organization_alias")?.map(str::to_owned),
        user_email: optional_text(index, row, "user_email")?.map(str::to_owned),
        api_key_alias: optional_text(index, row, "api_key_alias")?.map(str::to_owned),
    })
}

fn required<'row>(
    index: usize,
    row: &'row BTreeMap<String, JsonData>,
    field: &'static str,
) -> Result<&'row JsonData, UsageRowError> {
    match row.get(field) {
        None | Some(JsonData::Null) => Err(UsageRowError::Missing { index, field }),
        Some(value) => Ok(value),
    }
}

fn text<'row>(
    index: usize,
    row: &'row BTreeMap<String, JsonData>,
    field: &'static str,
) -> Result<&'row str, UsageRowError> {
    match required(index, row, field)? {
        JsonData::String(text) => Ok(text.as_str()),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => Err(UsageRowError::WrongType { index, field }),
    }
}

fn optional_text<'row>(
    index: usize,
    row: &'row BTreeMap<String, JsonData>,
    field: &'static str,
) -> Result<Option<&'row str>, UsageRowError> {
    match row.get(field) {
        None | Some(JsonData::Null) => Ok(None),
        Some(JsonData::String(text)) => Ok(Some(text.as_str())),
        Some(
            JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) | JsonData::Object(_),
        ) => Err(UsageRowError::WrongType { index, field }),
    }
}

fn integer(
    index: usize,
    row: &BTreeMap<String, JsonData>,
    field: &'static str,
) -> Result<i64, UsageRowError> {
    match required(index, row, field)? {
        JsonData::Number(number) => number
            .as_i64()
            .ok_or(UsageRowError::WrongType { index, field }),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::String(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => Err(UsageRowError::WrongType { index, field }),
    }
}

fn decimal(
    index: usize,
    row: &BTreeMap<String, JsonData>,
    field: &'static str,
) -> Result<Decimal, UsageRowError> {
    match required(index, row, field)? {
        JsonData::Number(number) => {
            Decimal::from_str(&number.to_string()).map_err(|source| UsageRowError::NotDecimal {
                index,
                field,
                source,
            })
        }
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::String(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => Err(UsageRowError::WrongType { index, field }),
    }
}
