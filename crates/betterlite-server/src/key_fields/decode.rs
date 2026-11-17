use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::private_json::safe_json::safe_json_loads;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::DecodeError;
use super::response::GenerateKeyResponse;
use super::values::LiteLLMObjectPermissionBase;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TypedValue {
    pub json: JsonData,
}

impl TypedValue {
    pub fn object(&self) -> Result<BTreeMap<String, JsonData>, DecodeError> {
        let JsonData::Object(fields) = &self.json else {
            return Err(DecodeError::NotAnObject);
        };
        Ok(fields.clone())
    }
}

// The owned outside-byte boundary: the logging crate's loader bounds the bytes at 1 MiB and the JSON at depth 64.
pub fn decode_key_request(bytes: &[u8]) -> Result<TypedValue, DecodeError> {
    let loaded = safe_json_loads(bytes)?;
    let value = serde_json::from_value::<JsonData>(loaded).map_err(DecodeError::Document)?;
    let JsonData::Object(_) = value else {
        return Err(DecodeError::NotAnObject);
    };
    Ok(TypedValue { json: value })
}

pub fn decode_key_response(bytes: &[u8]) -> Result<GenerateKeyResponse, DecodeError> {
    let value = decode_key_request(bytes)?;
    let response = GenerateKeyResponse::read(&value)?;
    Ok(response)
}

// The two pydantic dumps of one key model: `exclude_unset` and the full dump with its defaults filled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeyProjection {
    pub supplied: BTreeMap<String, JsonData>,
    pub resolved: BTreeMap<String, JsonData>,
}

// A patch field serializes only what the caller supplied, so the object is the pydantic `exclude_unset` dump.
pub(crate) fn to_object<T>(value: &T) -> Result<BTreeMap<String, JsonData>, DecodeError>
where
    T: Serialize,
{
    let JsonData::Object(fields) = to_json(value)? else {
        return Err(DecodeError::NotAnObject);
    };
    Ok(fields)
}

pub(crate) fn to_json<T>(value: &T) -> Result<JsonData, DecodeError>
where
    T: Serialize,
{
    let encoded = serde_json::to_value(value).map_err(DecodeError::Document)?;
    serde_json::from_value(encoded).map_err(DecodeError::Document)
}

pub(crate) fn decode_group<T>(fields: &BTreeMap<String, JsonData>) -> Result<T, DecodeError>
where
    T: DeserializeOwned,
{
    serde_path_to_error::deserialize(serde_json::to_value(fields).map_err(DecodeError::Document)?)
        .map_err(DecodeError::Fields)
}

pub(crate) fn resolved(
    defaults: &[&str],
    groups: &[&BTreeMap<String, JsonData>],
) -> Result<BTreeMap<String, JsonData>, DecodeError> {
    let mut fields = BTreeMap::new();
    for text in defaults {
        let JsonData::Object(part) =
            serde_json::from_str::<JsonData>(text).map_err(DecodeError::Document)?
        else {
            return Err(DecodeError::NotAnObject);
        };
        fields.extend(part);
    }
    for group in groups {
        fields.extend((*group).clone());
    }
    Ok(fields)
}

// A supplied nested permission model dumps all twelve fields, so the parent's full dump expands it here.
pub(crate) fn project(
    defaults: &[&str],
    groups: &[&BTreeMap<String, JsonData>],
    object_permission: Option<&LiteLLMObjectPermissionBase>,
) -> Result<KeyProjection, DecodeError> {
    let mut full = resolved(defaults, groups)?;
    if let Some(permission) = object_permission {
        full.insert("object_permission".to_owned(), permission.resolved_json()?);
    }
    let mut supplied = BTreeMap::new();
    for group in groups {
        supplied.extend((*group).clone());
    }
    strip_unset_windows(&mut supplied);
    Ok(KeyProjection {
        supplied,
        resolved: full,
    })
}

// exclude_unset drops the reset_at a typed window fills in when the caller sent none.
fn strip_unset_windows(supplied: &mut BTreeMap<String, JsonData>) {
    let Some(JsonData::Array(windows)) = supplied.get_mut("budget_limits") else {
        return;
    };
    for window in windows {
        let JsonData::Object(fields) = window else {
            continue;
        };
        if fields.get("reset_at") == Some(&JsonData::Null) {
            fields.remove("reset_at");
        }
    }
}

// The before field validators turn an empty string into None and keep the key in fields_set.
pub(crate) fn blank_to_null(fields: &mut BTreeMap<String, JsonData>, names: &[&str]) {
    for name in names {
        if matches!(fields.get(*name), Some(JsonData::String(text)) if text.is_empty()) {
            fields.insert((*name).to_owned(), JsonData::Null);
        }
    }
}

// The before model validator of UpdateKeyRequest drops a blank team_id from the mapping, so fields_set loses it.
pub(crate) fn drop_blank(fields: &mut BTreeMap<String, JsonData>, name: &str) {
    if matches!(fields.get(name), Some(JsonData::String(text)) if text.is_empty()) {
        fields.remove(name);
    }
}
