use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::{JsonData, Timestamp};

use super::base::KeyRequestBase;
use super::decode::{KeyProjection, TypedValue, blank_to_null, decode_group, project, to_object};
use super::defaults::{BASE_DEFAULTS, KEY_DEFAULTS, RESPONSE_DEFAULTS};
use super::error::DecodeError;

// The strings LiteLLM's before validator JSON-decodes ahead of field validation.
const STORED_JSON_FIELDS: &str = "metadata aliases config permissions model_max_budget budget_fallbacks router_settings budget_limits";

nullable_record! {
    (Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)
    pub struct ResponseFields {
        key: String,
        key_name: String,
        key_type: String,
        expires: Timestamp,
        user_id: String,
        token_id: String,
        organization_id: String,
        project_id: String,
        litellm_budget_table: JsonData,
        token: String,
        created_by: String,
        updated_by: String,
        created_at: Timestamp,
        updated_at: Timestamp,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateKeyResponse {
    pub base: KeyRequestBase,
    pub response: ResponseFields,
}

impl GenerateKeyResponse {
    pub fn read(value: &TypedValue) -> Result<Self, DecodeError> {
        let mut fields = value.object()?;
        let token = fields.get("token").cloned().unwrap_or(JsonData::Null);
        if token != JsonData::Null {
            fields.insert("key".to_owned(), token);
        }
        decode_stored_strings(&mut fields)?;
        blank_to_null(&mut fields, &["max_budget"]);
        let response = Self {
            base: KeyRequestBase::decode(&fields)?,
            response: decode_group(&fields)?,
        };
        if !matches!(response.response.key, Some(Some(_))) {
            return Err(DecodeError::MissingResponseKey);
        }
        Ok(response)
    }

    pub fn project(&self) -> Result<KeyProjection, DecodeError> {
        let base = to_object(&self.base.base)?;
        let key = to_object(&self.base.key_fields)?;
        let response = to_object(&self.response)?;
        project(
            &[BASE_DEFAULTS, KEY_DEFAULTS, RESPONSE_DEFAULTS],
            &[&base, &key, &response],
            self.base.object_permission(),
        )
    }
}

// A malformed stored string fails with LiteLLM's field-specific dictionary diagnostic.
fn decode_stored_strings(fields: &mut BTreeMap<String, JsonData>) -> Result<(), DecodeError> {
    for name in STORED_JSON_FIELDS.split(' ') {
        let Some(JsonData::String(text)) = fields.get(name) else {
            continue;
        };
        let decoded = serde_json::from_str::<JsonData>(text).map_err(|source| {
            DecodeError::InvalidDictionary {
                field: name.to_owned(),
                source,
            }
        })?;
        fields.insert(name.to_owned(), decoded);
    }
    Ok(())
}
