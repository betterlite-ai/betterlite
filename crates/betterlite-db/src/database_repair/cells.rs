use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_models::savings_evidence::billed_at::decode_billed_at;
use chrono::NaiveDateTime;
use csv::StringRecord;
use rust_decimal::Decimal;

use crate::budget_permission_rows::JsonData;
use crate::managed_resource_rows::check_depth;

use super::{MAX_JSON_DEPTH, TypedRow};

const NULL_CELL: &str = "NULL";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationTokenRow {
    pub token: String,
    pub key_name: Option<String>,
    pub key_alias: Option<String>,
    pub soft_budget_cooldown: Option<bool>,
    pub spend: Option<Decimal>,
    pub expires: Option<NaiveDateTime>,
    pub models: Option<Vec<String>>,
    pub aliases: Option<JsonData>,
    pub config: Option<JsonData>,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub permissions: Option<JsonData>,
    pub max_parallel_requests: Option<i64>,
    pub metadata: Option<JsonData>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub max_budget: Option<Decimal>,
    pub budget_duration: Option<String>,
    pub budget_reset_at: Option<NaiveDateTime>,
    pub allowed_cache_controls: Option<Vec<String>>,
    pub model_spend: Option<JsonData>,
    pub model_max_budget: Option<JsonData>,
    pub budget_id: Option<String>,
    pub blocked: Option<bool>,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub allowed_routes: Option<Vec<String>>,
    pub object_permission_id: Option<String>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub organization_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RowError {
    #[error("the CSV row has no {name} value")]
    MissingColumn { name: &'static str },
    #[error("the {name} value {value:?} is not a valid {kind}")]
    InvalidValue {
        name: &'static str,
        value: Box<str>,
        kind: &'static str,
    },
    #[error("the {name} value is a JSON document deeper than {MAX_JSON_DEPTH}")]
    TooDeep { name: &'static str },
}

pub struct HeaderIndex {
    positions: BTreeMap<String, usize>,
}

impl HeaderIndex {
    #[must_use]
    pub fn read(headers: &StringRecord) -> Self {
        let mut positions = BTreeMap::new();
        for (index, name) in headers.iter().enumerate() {
            positions.insert(name.to_owned(), index);
        }
        Self { positions }
    }
}

struct Cells<'row> {
    columns: &'row HeaderIndex,
    record: &'row StringRecord,
}

impl Cells<'_> {
    fn required(&self, name: &'static str) -> Result<&str, RowError> {
        let position = self
            .columns
            .positions
            .get(name)
            .ok_or(RowError::MissingColumn { name })?;
        self.record
            .get(*position)
            .ok_or(RowError::MissingColumn { name })
    }

    fn value(&self, name: &'static str) -> Result<Option<&str>, RowError> {
        if !self.columns.positions.contains_key(name) {
            return Err(RowError::MissingColumn { name });
        }
        Ok(self.optional_value(name))
    }

    fn optional_value(&self, name: &str) -> Option<&str> {
        let position = self.columns.positions.get(name)?;
        self.record
            .get(*position)
            .filter(|text| !text.is_empty() && *text != NULL_CELL)
    }

    fn boolean(&self, name: &'static str) -> Result<Option<bool>, RowError> {
        let Some(text) = self.value(name)? else {
            return Ok(None);
        };
        Ok(Some(text.to_lowercase() == "true"))
    }

    fn decimal(&self, name: &'static str) -> Result<Option<Decimal>, RowError> {
        let Some(text) = self.value(name)? else {
            return Ok(None);
        };
        let value = Decimal::from_str(text.trim()).ok();
        value
            .map(Some)
            .ok_or_else(|| invalid(name, text, "decimal"))
    }

    fn integer(&self, name: &'static str) -> Result<Option<i64>, RowError> {
        let Some(text) = self.value(name)? else {
            return Ok(None);
        };
        if !text.chars().all(|digit| digit.is_ascii_digit()) {
            return Ok(None);
        }
        let value = text.parse::<i64>().ok();
        value
            .map(Some)
            .ok_or_else(|| invalid(name, text, "integer"))
    }

    fn timestamp(&self, name: &'static str) -> Result<Option<NaiveDateTime>, RowError> {
        let Some(text) = self.value(name)? else {
            return Ok(None);
        };
        Ok(decode_billed_at(text).map(|decoded| decoded.naive_utc()))
    }

    fn json(&self, name: &'static str) -> Result<Option<JsonData>, RowError> {
        let Some(text) = self.value(name)? else {
            return Ok(None);
        };
        parse_json(name, text).map(Some)
    }

    fn strings(&self, name: &'static str) -> Result<Option<Vec<String>>, RowError> {
        let Some(text) = self.value(name)? else {
            return Ok(None);
        };
        Ok(Some(parse_list(text)))
    }
}

fn invalid(name: &'static str, text: &str, kind: &'static str) -> RowError {
    RowError::InvalidValue {
        name,
        value: text.to_owned().into_boxed_str(),
        kind,
    }
}

fn parse_json(name: &'static str, text: &str) -> Result<JsonData, RowError> {
    // the shared scanner reports only TooDeep, before serde walks the document
    if check_depth(text).is_err() {
        return Err(RowError::TooDeep { name });
    }
    let Ok(value) = serde_json::from_str(text) else {
        return Err(invalid(name, text, "JSON"));
    };
    Ok(value)
}

fn parse_list(text: &str) -> Vec<String> {
    let Some(content) = text
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return Vec::new();
    };
    if content.is_empty() {
        return Vec::new();
    }
    content
        .split(',')
        .map(|item| item.trim().to_owned())
        .collect()
}

#[must_use]
pub(crate) fn decode_row(columns: &HeaderIndex, record: &StringRecord) -> TypedRow {
    let cells = Cells { columns, record };
    let Ok(token) = cells.required("token") else {
        return TypedRow {
            token: None,
            outcome: Err(RowError::MissingColumn { name: "token" }),
        };
    };
    let outcome = coerce(&cells, token);
    TypedRow {
        token: Some(token.to_owned()),
        outcome,
    }
}

fn coerce(cells: &Cells<'_>, token: &str) -> Result<VerificationTokenRow, RowError> {
    let row = VerificationTokenRow {
        token: token.to_owned(),
        key_name: cells.value("key_name")?.map(ToOwned::to_owned),
        key_alias: cells.value("key_alias")?.map(ToOwned::to_owned),
        soft_budget_cooldown: cells.boolean("soft_budget_cooldown")?,
        spend: cells.decimal("spend")?,
        expires: cells.timestamp("expires")?,
        models: cells.strings("models")?,
        aliases: cells.json("aliases")?,
        config: cells.json("config")?,
        user_id: cells.value("user_id")?.map(ToOwned::to_owned),
        team_id: cells.optional_value("team_id").map(ToOwned::to_owned),
        permissions: cells.json("permissions")?,
        max_parallel_requests: cells.integer("max_parallel_requests")?,
        metadata: cells.json("metadata")?,
        tpm_limit: cells.integer("tpm_limit")?,
        rpm_limit: cells.integer("rpm_limit")?,
        max_budget: cells.decimal("max_budget")?,
        budget_duration: cells.value("budget_duration")?.map(ToOwned::to_owned),
        budget_reset_at: cells.timestamp("budget_reset_at")?,
        allowed_cache_controls: cells.strings("allowed_cache_controls")?,
        model_spend: cells.json("model_spend")?,
        model_max_budget: cells.json("model_max_budget")?,
        budget_id: cells.value("budget_id")?.map(ToOwned::to_owned),
        blocked: cells.boolean("blocked")?,
        created_at: cells.timestamp("created_at")?,
        updated_at: cells.timestamp("updated_at")?,
        allowed_routes: cells.strings("allowed_routes")?,
        object_permission_id: cells.value("object_permission_id")?.map(ToOwned::to_owned),
        created_by: cells.value("created_by")?.map(ToOwned::to_owned),
        updated_by: cells.value("updated_by")?.map(ToOwned::to_owned),
        organization_id: cells.value("organization_id")?.map(ToOwned::to_owned),
    };
    Ok(row)
}
