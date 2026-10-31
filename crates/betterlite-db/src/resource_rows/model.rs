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

// A string that fails json.loads keeps its original text, so the named field fails type validation instead of being replaced.
fn decode_json_string_field(object: &mut BTreeMap<String, JsonData>, field: &str) {
    let Some(JsonData::String(text)) = object.get(field) else {
        return;
    };
    if let Ok(parsed) = serde_json::from_str::<JsonData>(text) {
        object.insert(field.to_owned(), parsed);
    }
}

impl TryFrom<JsonData> for ProxyModelTable {
    type Error = ResourceRowsError;
    fn try_from(mut value: JsonData) -> Result<Self, Self::Error> {
        if let JsonData::Object(object) = &mut value {
            decode_json_string_field(object, "litellm_params");
            decode_json_string_field(object, "model_info");
        }
        Ok(Self {
            fields: convert(&value)?,
        })
    }
}
