use std::collections::BTreeMap;
use std::str::FromStr as _;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use csv::StringRecord;
use rust_decimal::Decimal;

use crate::budget_permission_rows::JsonData;
use crate::managed_resource_rows::check_depth;

use super::{MAX_JSON_DEPTH, TypedRow};

const NULL_CELL: &str = "NULL";

const AWARE_FORMATS: [&str; 4] = [
    "%Y-%m-%dT%H:%M:%S%.f%#z",
    "%Y-%m-%d %H:%M:%S%.f%#z",
    "%Y-%m-%dT%H:%M%#z",
    "%Y-%m-%d %H:%M%#z",
];

const NAIVE_FORMATS: [&str; 4] = [
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%dT%H:%M",
    "%Y-%m-%d %H:%M",
];

const DATE_FORMATS: [&str; 2] = ["%Y-%m-%d", "%Y%m%d"];

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
        let Some(position) = self.columns.positions.get(name) else {
            return Err(RowError::MissingColumn { name });
        };
        let cell = self
            .record
            .get(*position)
            .filter(|text| !text.is_empty() && *text != NULL_CELL);
        Ok(cell)
    }

    fn boolean(&self, name: &'static str) -> Result<Option<bool>, RowError> {
        if let Some(text) = self.value(name)? {
            return Ok(Some(text.to_lowercase() == "true"));
        }
        Ok(None)
    }

    fn optional_value(&self, name: &str) -> Option<&str> {
        let position = self.columns.positions.get(name)?;
        self.record
            .get(*position)
            .filter(|text| !text.is_empty() && *text != NULL_CELL)
    }

    fn decimal(&self, name: &'static str) -> Result<Option<Decimal>, RowError> {
        let Some(text) = self.value(name)? else {
            return Ok(None);
        };
        let Ok(value) = Decimal::from_str(text.trim()) else {
            return Err(RowError::InvalidValue {
                name,
                value: text.to_owned().into_boxed_str(),
                kind: "decimal",
            });
        };
        Ok(Some(value))
    }

    fn integer(&self, name: &'static str) -> Result<Option<i64>, RowError> {
        let Some(text) = self.value(name)? else {
            return Ok(None);
        };
        if !text.chars().all(|digit| digit.is_ascii_digit()) {
            return Ok(None);
        }
        let Ok(value) = text.parse::<i64>() else {
            return Err(RowError::InvalidValue {
                name,
                value: text.to_owned().into_boxed_str(),
                kind: "integer",
            });
        };
        Ok(Some(value))
    }

    fn timestamp(&self, name: &'static str) -> Result<Option<NaiveDateTime>, RowError> {
        let Some(text) = self.value(name)? else {
            return Ok(None);
        };
        let normalized = text.replace('Z', "+00:00");
        let aware = AWARE_FORMATS.iter().find_map(|format| {
            DateTime::parse_from_str(&normalized, format)
                .ok()
                .map(|value| value.with_timezone(&Utc).naive_utc())
        });
        let naive = NAIVE_FORMATS
            .iter()
            .find_map(|format| NaiveDateTime::parse_from_str(&normalized, format).ok());
        let date = DATE_FORMATS
            .iter()
            .find_map(|format| NaiveDate::parse_from_str(&normalized, format).ok());
        Ok(aware
            .or(naive)
            .or_else(|| date.and_then(|value| value.and_hms_opt(0, 0, 0))))
    }

    fn json(&self, name: &'static str) -> Result<Option<JsonData>, RowError> {
        let Some(text) = self.value(name)? else {
            return Ok(None);
        };
        // the shared scanner reports only TooDeep, before serde walks the document
        if check_depth(text).is_err() {
            return Err(RowError::TooDeep { name });
        }
        let Ok(value) = serde_json::from_str::<JsonData>(text) else {
            return Err(RowError::InvalidValue {
                name,
                value: text.to_owned().into_boxed_str(),
                kind: "JSON",
            });
        };
        Ok(Some(value))
    }

    fn strings(&self, name: &'static str) -> Result<Option<Vec<String>>, RowError> {
        let Some(text) = self.value(name)? else {
            return Ok(None);
        };
        let Some(content) = text
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))
        else {
            return Ok(Some(Vec::new()));
        };
        if content.is_empty() {
            return Ok(Some(Vec::new()));
        }
        let items: Vec<String> = content
            .split(',')
            .map(|item| item.trim().to_owned())
            .collect();
        Ok(Some(items))
    }
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
