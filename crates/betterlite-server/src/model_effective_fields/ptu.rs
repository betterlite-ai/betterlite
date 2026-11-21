use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::ptu_pricing::{PtuModelInfo, ptu_config_error};
use betterlite_models::tiered_pricing::Scalar;
use serde::de::IgnoredAny;

use crate::jobs::ptu::attribution::PtuAttribution;

use super::error::ModelFieldError;

// LiteLLM's PTU_MODEL_INFO_FIELDS, in its order, which the refusal lists
const PTU_FIELDS: [&str; 4] = [
    "ptu_count",
    "cost_per_ptu_per_hour",
    "ptu_effective_from",
    "ptu_effective_to",
];

// empty while attribution is off, so disabling pauses PTU instead of erasing what an earlier opt-in stored
#[must_use]
pub fn explicitly_cleared_ptu_fields(
    incoming: Option<&JsonObject>,
    attribution: &dyn PtuAttribution,
) -> Vec<&'static str> {
    let Some(model_info) = incoming else {
        return Vec::new();
    };
    if !attribution.enabled() {
        return Vec::new();
    }
    PTU_FIELDS
        .into_iter()
        .filter(|field| matches!(model_info.get(*field), Some(Json::Null)))
        .collect()
}

// the PTU invariant holds over the model_info the patch would store, not over the fields it happened to send
#[must_use]
pub fn merged_ptu_model_info(
    stored: &JsonObject,
    incoming: Option<&JsonObject>,
    attribution: &dyn PtuAttribution,
) -> JsonObject {
    let cleared = explicitly_cleared_ptu_fields(incoming, attribution);
    let mut merged = JsonObject::new();
    for (field, value) in stored.iter().chain(incoming.into_iter().flatten()) {
        if !matches!(value, Json::Null) && !cleared.contains(&field.as_str()) {
            merged.insert(field.clone(), value.clone());
        }
    }
    merged
}

// judged on the request's own model_info, so an unrelated patch of a model that still stores PTU terms passes
pub fn check_ptu_fields_allowed(
    incoming: &JsonObject,
    attribution: &dyn PtuAttribution,
) -> Result<(), ModelFieldError> {
    if attribution.enabled() {
        return Ok(());
    }
    let supplied: Vec<&'static str> = PTU_FIELDS
        .into_iter()
        .filter(|field| {
            incoming
                .get(*field)
                .is_some_and(|value| !matches!(value, Json::Null))
        })
        .collect();
    if supplied.is_empty() {
        return Ok(());
    }
    Err(ModelFieldError::PtuDisabled { supplied })
}

pub fn check_ptu_model_info(model_info: &JsonObject) -> Result<(), ModelFieldError> {
    let mut scalars = BTreeMap::new();
    for field in PTU_FIELDS.into_iter().chain(["team_id"]) {
        if let Some(value) = model_info.get(field) {
            scalars.insert(field.to_owned(), scalar(value));
        }
    }
    match ptu_config_error(&PtuModelInfo::from(&scalars), None) {
        Some(message) => Err(ModelFieldError::PtuInvalid { message }),
        None => Ok(()),
    }
}

// the PTU rules read only the scalars, so a list or object keeps just its shape
fn scalar(value: &Json) -> Scalar {
    match value {
        Json::Null => Scalar::Null,
        Json::Bool(flag) => Scalar::Bool(*flag),
        Json::Number(number) => Scalar::Number(number.clone()),
        Json::Text(text) => Scalar::Text(text.clone()),
        Json::List(items) => Scalar::Array(items.iter().map(|_item| IgnoredAny).collect()),
        Json::Object(_) => Scalar::Object(IgnoredAny),
    }
}
