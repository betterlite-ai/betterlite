use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::TeamContractError;
use crate::key_fields::values::LiteLLMObjectPermissionBase;
use crate::proxy_request_contracts::{ContractError, Fields, decode};
use crate::router_weight_references::weights::decode_router_weights;

pub const MAX_BULK_TEAM_MEMBER_DELETES: usize = 500;

pub const MAX_BULK_BUDGET_UPDATES: usize = 500;

pub fn decode_request<T>(value: &JsonData) -> Result<T, TeamContractError>
where
    T: DeserializeOwned,
{
    Ok(decode(object(value)?)?)
}

pub(crate) fn object(value: &JsonData) -> Result<&Fields, TeamContractError> {
    let JsonData::Object(fields) = value else {
        return Err(TeamContractError::Validation {
            message: "Input should be a valid dictionary or object".to_owned(),
        });
    };
    Ok(fields)
}

pub(crate) fn to_data<T>(value: &T) -> Result<JsonData, TeamContractError>
where
    T: Serialize + ?Sized,
{
    serde_json::from_value(serde_json::to_value(value).map_err(ContractError::from)?)
        .map_err(ContractError::from)
        .map_err(TeamContractError::from)
}

pub(crate) fn to_object<T>(value: &T) -> Result<Fields, TeamContractError>
where
    T: Serialize + ?Sized,
{
    let data = to_data(value)?;
    let JsonData::Object(fields) = data else {
        return Err(TeamContractError::Validation {
            message: "the team contract does not serialize to an object".to_owned(),
        });
    };
    Ok(fields)
}

pub(crate) fn request_object<T>(
    request: &T,
    permission: Option<&LiteLLMObjectPermissionBase>,
) -> Result<Fields, TeamContractError>
where
    T: Serialize + ?Sized,
{
    let mut fields = to_object(request)?;
    let Some(found_permission) = permission else {
        return Ok(fields);
    };
    let resolved = found_permission.resolved_json()?;
    fields.insert("object_permission".to_owned(), resolved);
    Ok(fields)
}

// The caller's fields survive as supplied: a full dump pruned against the object the caller sent.
pub trait RequestProjection: Serialize {
    fn supplied_document(&self) -> &JsonData;

    fn supplied_permission(&self) -> Option<&LiteLLMObjectPermissionBase>;

    fn resolved(&self) -> Result<JsonData, TeamContractError> {
        let fields = request_object(self, self.supplied_permission())?;
        Ok(JsonData::Object(fields))
    }

    fn supplied(&self) -> Result<JsonData, TeamContractError> {
        let mut full = self.resolved()?;
        prune(&mut full, self.supplied_document());
        Ok(full)
    }
}

pub(crate) fn prune(full: &mut JsonData, raw: &JsonData) {
    match full {
        JsonData::Object(full_fields) => {
            if let JsonData::Object(raw_fields) = raw {
                prune_object(full_fields, raw_fields);
            }
        }
        JsonData::Array(full_items) => {
            if let JsonData::Array(raw_items) = raw {
                for (item, raw_item) in full_items.iter_mut().zip(raw_items.iter()) {
                    prune(item, raw_item);
                }
            }
        }
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) => {}
    }
}

fn prune_object(full: &mut Fields, raw: &Fields) {
    full.retain(|key, _value| raw.contains_key(key));
    for (key, value) in full {
        if let Some(raw_value) = raw.get(key) {
            prune(value, raw_value);
        }
    }
}

pub(crate) fn blank_team_id_to_null(fields: &mut Fields) {
    let blank = matches!(
        fields.get("team_id"),
        Some(JsonData::String(text)) if text.chars().all(is_strip_space)
    );
    if blank {
        fields.insert("team_id".to_owned(), JsonData::Null);
    }
}

// str.strip() also strips U+001C-U+001F, which Rust's is_whitespace() leaves alone
const fn is_strip_space(character: char) -> bool {
    match character {
        '\u{1c}'..='\u{1f}' => true,
        _ => character.is_whitespace(),
    }
}

pub(crate) fn normalize_budget_aliases(fields: &mut Fields) {
    let Some(JsonData::Object(entries)) = fields.get_mut("model_max_budget") else {
        return;
    };
    for entry in entries.values_mut() {
        let JsonData::Object(entry_fields) = entry else {
            continue;
        };
        if let Some(limit) = entry_fields.remove("budget_limit") {
            entry_fields.insert("max_budget".to_owned(), limit);
        }
        if let Some(period) = entry_fields.remove("time_period") {
            entry_fields.insert("budget_duration".to_owned(), period);
        }
    }
}

pub(crate) fn validate_router_settings(
    settings: Option<&BTreeMap<String, JsonData>>,
) -> Result<(), TeamContractError> {
    let Some(found_settings) = settings else {
        return Ok(());
    };
    let text = serde_json::to_string(found_settings).map_err(ContractError::from)?;
    let value: Json = serde_json::from_str(&text).map_err(ContractError::from)?;
    decode_router_weights(&value).map_err(|source| TeamContractError::Validation {
        message: source.to_string(),
    })?;
    Ok(())
}

pub(crate) fn check_member_count(length: usize, limit: usize) -> Result<(), TeamContractError> {
    if length == 0 {
        return Err(TeamContractError::Validation {
            message: "Tuple should have at least 1 item after validation, not 0".to_owned(),
        });
    }
    if length > limit {
        let message =
            format!("Tuple should have at most {limit} items after validation, not {length}");
        return Err(TeamContractError::Validation { message });
    }
    Ok(())
}
