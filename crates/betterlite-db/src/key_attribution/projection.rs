use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::budget_permission_rows::JsonData;

use super::metadata::KeyMetadata;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportFields {
    #[serde(rename = "api_key")]
    pub key_column: String,
    pub alias: String,
    pub team_id: String,
    pub user_email: String,
}

impl Default for ExportFields {
    fn default() -> Self {
        Self {
            key_column: "api_key".to_owned(),
            alias: "api_key_alias".to_owned(),
            team_id: "team_id".to_owned(),
            user_email: "user_email".to_owned(),
        }
    }
}

#[must_use]
pub fn missing_keys(
    rows: &[BTreeMap<String, JsonData>],
    fields: &ExportFields,
) -> BTreeSet<String> {
    rows.iter()
        .filter_map(|row| {
            let Some(JsonData::String(key)) = row.get(&fields.key_column) else {
                return None;
            };
            if key.is_empty() || !alias_is_missing(row.get(&fields.alias)) {
                return None;
            }
            Some(key.clone())
        })
        .collect()
}

fn alias_is_missing(alias: Option<&JsonData>) -> bool {
    match alias {
        None | Some(JsonData::Null) => true,
        Some(JsonData::String(text)) => text.is_empty(),
        Some(_) => false,
    }
}

#[must_use]
pub fn row_with_recovered_fields(
    row: &BTreeMap<String, JsonData>,
    recovered: &BTreeMap<String, KeyMetadata>,
    fields: &ExportFields,
) -> BTreeMap<String, JsonData> {
    let Some(meta) = row.get(&fields.key_column).and_then(|key| match key {
        JsonData::String(key) => recovered.get(key),
        _ => None,
    }) else {
        return row.clone();
    };
    let mut projected = row.clone();
    let alias = match meta.key_alias.as_deref().filter(|alias| !alias.is_empty()) {
        Some(alias) => JsonData::String(alias.to_owned()),
        None => row.get(&fields.alias).cloned().unwrap_or(JsonData::Null),
    };
    let team = match meta.team_id.as_deref().filter(|team| !team.is_empty()) {
        Some(team) => JsonData::String(team.to_owned()),
        None => row.get(&fields.team_id).cloned().unwrap_or(JsonData::Null),
    };
    let email = row
        .get(&fields.user_email)
        .filter(|value| truthy(value))
        .cloned()
        .or_else(|| meta.user_email.clone().map(JsonData::String))
        .unwrap_or(JsonData::Null);
    projected.insert(fields.alias.clone(), alias);
    projected.insert(fields.team_id.clone(), team);
    projected.insert(fields.user_email.clone(), email);
    projected
}

#[must_use]
pub fn project_rows(
    rows: &[BTreeMap<String, JsonData>],
    recovered: &BTreeMap<String, KeyMetadata>,
    fields: &ExportFields,
) -> Vec<BTreeMap<String, JsonData>> {
    rows.iter()
        .map(|row| row_with_recovered_fields(row, recovered, fields))
        .collect()
}

fn truthy(value: &JsonData) -> bool {
    match value {
        JsonData::Null => false,
        JsonData::Bool(flag) => *flag,
        JsonData::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        JsonData::String(text) => !text.is_empty(),
        JsonData::Array(values) => !values.is_empty(),
        JsonData::Object(fields) => !fields.is_empty(),
    }
}
