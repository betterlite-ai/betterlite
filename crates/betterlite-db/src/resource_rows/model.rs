use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{ResourceRowsError, convert};
use crate::budget_permission_rows::{JsonData, Timestamp};
use crate::identity_rows::Payload;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFields {
    pub model_id: String,
    pub model_name: String,
    pub litellm_params: Payload,
    #[serde(default)]
    pub model_info: Option<Payload>,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub updated_at: Option<Timestamp>,
    #[serde(default)]
    pub updated_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "JsonData")]
pub struct ProxyModelTable {
    #[serde(flatten)]
    pub fields: ModelFields,
}

// A string that fails to parse as JSON is left alone, so field validation rejects it later.
fn decode_json_string_field(
    object: &mut BTreeMap<String, JsonData>,
    field: &str,
) -> Result<(), ResourceRowsError> {
    let Some(JsonData::String(text)) = object.get(field) else {
        return Ok(());
    };
    match super::parse_data(text.as_bytes()) {
        Ok(parsed) => {
            object.insert(field.to_owned(), parsed);
        }
        Err(ResourceRowsError::Json(_)) => {}
        Err(other) => return Err(other),
    }
    Ok(())
}

pub(crate) fn decode_nested_json_fields(value: &mut JsonData) -> Result<(), ResourceRowsError> {
    if let JsonData::Object(object) = value {
        decode_json_string_field(object, "litellm_params")?;
        decode_json_string_field(object, "model_info")?;
    }
    Ok(())
}

impl TryFrom<JsonData> for ProxyModelTable {
    type Error = ResourceRowsError;
    fn try_from(mut value: JsonData) -> Result<Self, Self::Error> {
        decode_nested_json_fields(&mut value)?;
        Ok(Self {
            fields: convert(&value)?,
        })
    }
}
