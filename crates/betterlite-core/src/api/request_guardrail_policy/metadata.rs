use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use crate::api::request_destination::RequestData;
use crate::api::request_guardrail_policy::error::RequestGuardrailPolicyError;
use crate::api::request_guardrail_policy::requested_guardrails::{
    RequestedGuardrail, requested_guardrail,
};
use crate::api::request_guardrail_policy::{GuardrailPolicySettings, RequestGuardrailPolicyInput};

pub(super) const GUARDRAILS: &str = "guardrails";
pub(super) const GUARDRAIL_CONFIG: &str = "guardrail_config";
pub(super) const INCLUDE_GUARDRAIL_RESPONSE: &str = "include_guardrail_response";
pub(super) const APPLIED_POLICIES: &str = "applied_policies";
pub(super) const POLICY_SOURCES: &str = "policy_sources";

// LiteLLM's get_or_create_metadata_bucket: a bucket that is absent or not an object becomes one
pub(super) fn bucket_mut<'data>(
    data: &'data mut RequestData,
    name: &str,
) -> Result<&'data mut IndexMap<String, Json>, RequestGuardrailPolicyError> {
    if !matches!(data.get(name), Some(Json::Object(_))) {
        data.insert(name.to_owned(), Json::Object(IndexMap::new()));
    }
    let Some(Json::Object(bucket)) = data.get_mut(name) else {
        return Err(RequestGuardrailPolicyError::MetadataBucket);
    };
    Ok(bucket)
}

pub(super) fn texts(
    items: Vec<Json>,
    field: &'static str,
) -> Result<Vec<String>, RequestGuardrailPolicyError> {
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        let Json::Text(name) = item else {
            return Err(RequestGuardrailPolicyError::NotTyped {
                field,
                expected: "a list of strings",
            });
        };
        names.push(name);
    }
    Ok(names)
}

pub(super) fn json_from_serialize<T>(value: &T) -> Result<Json, RequestGuardrailPolicyError>
where
    T: Serialize + ?Sized,
{
    match serde_json::to_string(value) {
        Ok(text) => serde_json::from_str(&text).map_err(RequestGuardrailPolicyError::Serialization),
        Err(error) => Err(RequestGuardrailPolicyError::Serialization(error)),
    }
}

pub(super) fn existing_string_list(
    metadata: &IndexMap<String, Json>,
    key: &'static str,
) -> Result<Vec<String>, RequestGuardrailPolicyError> {
    match metadata.get(key) {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(items)) => texts(items.clone(), key),
        Some(_) => Err(RequestGuardrailPolicyError::NotTyped {
            field: key,
            expected: "a list of strings",
        }),
    }
}

pub(super) fn policy_sources(metadata: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    if let Some(Json::Object(sources)) = metadata.get(POLICY_SOURCES) {
        return sources.clone();
    }
    IndexMap::new()
}

pub(super) const fn require_premium(premium_user: bool) -> Result<(), RequestGuardrailPolicyError> {
    if premium_user {
        return Ok(());
    }
    Err(RequestGuardrailPolicyError::NotPremium)
}

pub(super) fn move_include_flag(
    data: &mut RequestData,
    name: &str,
) -> Result<(), RequestGuardrailPolicyError> {
    let Some(value) = data.shift_remove(INCLUDE_GUARDRAIL_RESPONSE) else {
        return Ok(());
    };
    let flag = match value {
        Json::Bool(flag) => flag,
        Json::Null => false,
        Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            return Err(RequestGuardrailPolicyError::NotTyped {
                field: INCLUDE_GUARDRAIL_RESPONSE,
                expected: "a boolean",
            });
        }
    };
    bucket_mut(data, name)?.insert(INCLUDE_GUARDRAIL_RESPONSE.to_owned(), Json::Bool(flag));
    Ok(())
}

// key first, then team and project, without duplicates; the write replaces what an earlier stage left
pub(super) fn add_direct_guardrails(
    input: &RequestGuardrailPolicyInput,
    data: &mut RequestData,
    name: &str,
    settings: &GuardrailPolicySettings,
) -> Result<(), RequestGuardrailPolicyError> {
    let mut combined = Vec::new();
    let mut seen = BTreeSet::new();
    for metadata in [
        &input.key_metadata,
        &input.team_metadata,
        &input.project_metadata,
    ] {
        if metadata.guardrails.is_empty() {
            continue;
        }
        require_premium(settings.premium_user)?;
        for guardrail in &metadata.guardrails {
            if seen.insert(guardrail.clone()) {
                combined.push(guardrail.clone());
            }
        }
    }
    if combined.is_empty() {
        return Ok(());
    }
    bucket_mut(data, name)?.insert(
        GUARDRAILS.to_owned(),
        Json::List(combined.into_iter().map(Json::Text).collect()),
    );
    Ok(())
}

pub(super) fn move_request_guardrails(
    data: &mut RequestData,
    name: &str,
) -> Result<(), RequestGuardrailPolicyError> {
    if let Some(value) = data.shift_remove(GUARDRAILS) {
        let field = moved_field(value, MovedKind::Guardrails)?;
        apply_moved_field(data, name, field)?;
    }
    if let Some(value) = data.shift_remove(GUARDRAIL_CONFIG) {
        let field = moved_field(value, MovedKind::Config)?;
        apply_moved_field(data, name, field)?;
    }
    Ok(())
}

enum MovedField {
    Guardrails(Option<Vec<RequestedGuardrail>>),
    Config(Option<IndexMap<String, Json>>),
}

#[derive(Clone, Copy)]
enum MovedKind {
    Guardrails,
    Config,
}

fn moved_field(value: Json, kind: MovedKind) -> Result<MovedField, RequestGuardrailPolicyError> {
    match kind {
        MovedKind::Guardrails => match value {
            Json::Null => Ok(MovedField::Guardrails(None)),
            Json::List(items) => {
                let guardrails = items
                    .into_iter()
                    .map(requested_guardrail)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(MovedField::Guardrails(Some(guardrails)))
            }
            Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
                Err(RequestGuardrailPolicyError::NotTyped {
                    field: GUARDRAILS,
                    expected: "a list of guardrail names or guardrail params objects",
                })
            }
        },
        MovedKind::Config => match value {
            Json::Null => Ok(MovedField::Config(None)),
            Json::Object(fields) => Ok(MovedField::Config(Some(fields))),
            Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                Err(RequestGuardrailPolicyError::NotTyped {
                    field: GUARDRAIL_CONFIG,
                    expected: "an object",
                })
            }
        },
    }
}

fn apply_moved_field(
    data: &mut RequestData,
    name: &str,
    field: MovedField,
) -> Result<(), RequestGuardrailPolicyError> {
    let bucket = bucket_mut(data, name)?;
    match field {
        MovedField::Guardrails(None) => {
            if !matches!(bucket.get(GUARDRAILS), Some(Json::List(_))) {
                bucket.insert(GUARDRAILS.to_owned(), Json::Null);
            }
        }
        MovedField::Guardrails(Some(guardrails)) => {
            let mut stored = match bucket.get(GUARDRAILS) {
                Some(Json::List(existing)) => existing.clone(),
                _ => Vec::new(),
            };
            for guardrail in &guardrails {
                stored.push(json_from_serialize(guardrail)?);
            }
            bucket.insert(GUARDRAILS.to_owned(), Json::List(stored));
        }
        MovedField::Config(None) => {
            if !matches!(bucket.get(GUARDRAIL_CONFIG), Some(Json::Object(_))) {
                bucket.insert(GUARDRAIL_CONFIG.to_owned(), Json::Null);
            }
        }
        MovedField::Config(Some(fields)) => {
            let mut stored = match bucket.shift_remove(GUARDRAIL_CONFIG) {
                Some(Json::Object(existing)) => existing,
                _ => IndexMap::new(),
            };
            stored.extend(fields);
            bucket.insert(GUARDRAIL_CONFIG.to_owned(), Json::Object(stored));
        }
    }
    Ok(())
}
