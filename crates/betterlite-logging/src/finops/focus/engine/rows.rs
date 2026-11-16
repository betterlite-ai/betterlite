use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::finops::focus::UsageData;
use rust_decimal::Decimal;

use crate::focus_records::transformer::UsageRow;

#[derive(Debug, thiserror::Error)]
pub enum UsageRowError {
    #[error("usage row {0} has no '{1}' value")]
    Missing(usize, &'static str),
    #[error("usage row {0} has a '{1}' value of the wrong type")]
    WrongType(usize, &'static str),
    #[error("usage row {0} has a '{1}' value that is not a decimal")]
    NotDecimal(usize, &'static str, #[source] rust_decimal::Error),
}

pub fn decode(rows: &UsageData) -> Result<Vec<UsageRow>, UsageRowError> {
    rows.iter()
        .enumerate()
        .map(|(index, row)| decode_row(index, row))
        .collect()
}

fn decode_row(index: usize, row: &BTreeMap<String, JsonData>) -> Result<UsageRow, UsageRowError> {
    let reader = Row { index, values: row };
    Ok(UsageRow {
        date: reader.text("date")?.to_owned(),
        api_key: reader.text("api_key")?.to_owned(),
        spend: reader.decimal("spend")?,
        api_requests: reader.integer("api_requests")?,
        user_id: reader.optional_text("user_id")?.map(str::to_owned),
        model: reader.optional_text("model")?.map(str::to_owned),
        model_group: reader.optional_text("model_group")?.map(str::to_owned),
        custom_llm_provider: reader
            .optional_text("custom_llm_provider")?
            .map(str::to_owned),
        team_id: reader.optional_text("team_id")?.map(str::to_owned),
        team_alias: reader.optional_text("team_alias")?.map(str::to_owned),
        organization_id: reader.optional_text("organization_id")?.map(str::to_owned),
        organization_alias: reader
            .optional_text("organization_alias")?
            .map(str::to_owned),
        user_email: reader.optional_text("user_email")?.map(str::to_owned),
        api_key_alias: reader.optional_text("api_key_alias")?.map(str::to_owned),
    })
}

struct Row<'row> {
    index: usize,
    values: &'row BTreeMap<String, JsonData>,
}

impl<'row> Row<'row> {
    fn required(&self, field: &'static str) -> Result<&'row JsonData, UsageRowError> {
        match self.values.get(field) {
            None | Some(JsonData::Null) => Err(UsageRowError::Missing(self.index, field)),
            Some(value) => Ok(value),
        }
    }

    fn text(&self, field: &'static str) -> Result<&'row str, UsageRowError> {
        let JsonData::String(text) = self.required(field)? else {
            return Err(UsageRowError::WrongType(self.index, field));
        };
        Ok(text.as_str())
    }

    fn optional_text(&self, field: &'static str) -> Result<Option<&'row str>, UsageRowError> {
        match self.values.get(field) {
            None | Some(JsonData::Null) => Ok(None),
            Some(JsonData::String(text)) => Ok(Some(text.as_str())),
            Some(_) => Err(UsageRowError::WrongType(self.index, field)),
        }
    }

    fn integer(&self, field: &'static str) -> Result<i64, UsageRowError> {
        let JsonData::Number(number) = self.required(field)? else {
            return Err(UsageRowError::WrongType(self.index, field));
        };
        number
            .as_i64()
            .ok_or(UsageRowError::WrongType(self.index, field))
    }

    fn decimal(&self, field: &'static str) -> Result<Decimal, UsageRowError> {
        let JsonData::Number(number) = self.required(field)? else {
            return Err(UsageRowError::WrongType(self.index, field));
        };
        Decimal::from_str(&number.to_string())
            .map_err(|source| UsageRowError::NotDecimal(self.index, field, source))
    }
}
