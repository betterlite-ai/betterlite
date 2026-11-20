use std::cmp::Ordering;
use std::collections::BTreeMap;

use betterlite_config::sections::team_fields::schema::MetadataSchemaField;
use betterlite_db::budget_permission_rows::JsonData;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::team_request_contracts::metadata::TeamMetadataFieldSchema;

pub const DEFAULT_REJECTED_MESSAGE: &str = "Team metadata failed validation.";

pub type MetadataMap = BTreeMap<String, JsonData>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TeamMetadataRequester {
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub user_role: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamMetadataOperation {
    Create,
    Update,
}

impl TeamMetadataOperation {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TeamMetadataValidationPayload {
    pub operation: TeamMetadataOperation,
    pub metadata: MetadataMap,
    pub existing_metadata: Option<MetadataMap>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub requester: TeamMetadataRequester,
}

// The six values validate_team_metadata_if_configured receives beside the registry.
pub struct MetadataRequest<'input> {
    pub operation: TeamMetadataOperation,
    pub metadata: &'input JsonData,
    pub existing_metadata: Option<&'input JsonData>,
    pub team_id: Option<&'input str>,
    pub team_alias: Option<&'input str>,
    pub requester: TeamMetadataRequester,
}

impl From<&MetadataSchemaField> for TeamMetadataFieldSchema {
    fn from(field: &MetadataSchemaField) -> Self {
        Self {
            key: field.key.clone(),
            label: field.label.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamMetadataValidationResult {
    pub valid: bool,
    pub error_message: Option<String>,
}

impl TeamMetadataValidationResult {
    // LiteLLM reads the callback result with model_validate, so pydantic's lax bool spellings count here.
    #[must_use]
    pub fn parse(value: &JsonData) -> Option<Self> {
        let JsonData::Object(entries) = value else {
            return None;
        };
        let valid = bool_field(entries.get("valid")?)?;
        let error_message = match entries.get("error_message") {
            None | Some(JsonData::Null) => None,
            Some(JsonData::String(message)) => Some(message.clone()),
            Some(JsonData::Bool(_))
            | Some(JsonData::Number(_))
            | Some(JsonData::Array(_))
            | Some(JsonData::Object(_)) => return None,
        };
        Some(Self {
            valid,
            error_message,
        })
    }
}

// pydantic's bool validator: true and false spellings, the numbers 0 and 1, and nothing else.
fn bool_field(value: &JsonData) -> Option<bool> {
    match value {
        JsonData::Bool(flag) => Some(*flag),
        JsonData::Number(number) => number_bool(number),
        JsonData::String(text) => text_bool(text),
        JsonData::Null | JsonData::Array(_) | JsonData::Object(_) => None,
    }
}

fn number_bool(number: &Number) -> Option<bool> {
    let real = number.as_f64()?;
    if real.partial_cmp(&0.0_f64) == Some(Ordering::Equal) {
        return Some(false);
    }
    if real.partial_cmp(&1.0_f64) == Some(Ordering::Equal) {
        return Some(true);
    }
    None
}

fn text_bool(text: &str) -> Option<bool> {
    if ["0", "off", "f", "false", "n", "no"]
        .into_iter()
        .any(|token| text.eq_ignore_ascii_case(token))
    {
        return Some(false);
    }
    if ["1", "on", "t", "true", "y", "yes"]
        .into_iter()
        .any(|token| text.eq_ignore_ascii_case(token))
    {
        return Some(true);
    }
    None
}

#[must_use]
pub fn build_payload(request: MetadataRequest<'_>) -> TeamMetadataValidationPayload {
    let MetadataRequest {
        operation,
        metadata,
        existing_metadata,
        team_id,
        team_alias,
        requester,
    } = request;
    TeamMetadataValidationPayload {
        operation,
        metadata: metadata_map(metadata).unwrap_or_default(),
        existing_metadata: existing_metadata.and_then(metadata_map),
        team_id: team_id.map(str::to_owned),
        team_alias: team_alias.map(str::to_owned),
        requester,
    }
}

// Only a JSON object is a dict to LiteLLM, so a list, string or null metadata becomes the empty mapping.
fn metadata_map(value: &JsonData) -> Option<MetadataMap> {
    match value {
        JsonData::Object(entries) => Some(entries.clone()),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => None,
    }
}
