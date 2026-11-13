use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Number;

use crate::resolved_key::auth::UserAPIKeyAuth;
use crate::user_roles::UserRole;

use super::error::{AuthLimitInputsError, ManagementEntity};

pub const ESTIMATED_OUTPUT_TOKENS_FIELD: &str = "default_estimated_output_tokens";
pub const ESTIMATE_PER_MODEL_FIELD: &str = "default_estimated_output_tokens_per_model";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
pub enum OutputTokenEstimateField {
    #[serde(rename = "metadata")]
    Metadata,
    #[serde(rename = "default_estimated_output_tokens")]
    Estimate,
    #[serde(rename = "default_estimated_output_tokens_per_model")]
    EstimatePerModel,
}

// The request members the admin gate reads, with model_fields_set saying which ones the caller sent.
#[derive(Clone, Copy, Debug)]
pub struct OutputTokenEstimateRequest<'request> {
    pub metadata: Option<&'request BTreeMap<String, JsonData>>,
    pub estimate: Option<i64>,
    pub estimate_per_model: Option<&'request BTreeMap<String, i64>>,
    pub model_fields_set: &'request BTreeSet<OutputTokenEstimateField>,
}

#[must_use]
pub fn get_estimated_output_tokens(auth: &UserAPIKeyAuth, model_name: Option<&str>) -> Option<i64> {
    let key_estimate = estimate_from_metadata(&auth.view.token.metadata.0, model_name);
    if key_estimate.is_some() {
        return key_estimate;
    }
    let team_metadata = auth.view.team_metadata.as_ref()?;
    estimate_from_metadata(&team_metadata.0, model_name)
}

pub fn enforce_estimate_admin_only(
    request: &OutputTokenEstimateRequest<'_>,
    existing_metadata: Option<&BTreeMap<String, JsonData>>,
    caller: &UserAPIKeyAuth,
    entity: ManagementEntity,
) -> Result<(), AuthLimitInputsError> {
    if caller.extra.user_role == Some(UserRole::ProxyAdmin) {
        return Ok(());
    }
    let empty = BTreeMap::new();
    let stored = existing_metadata.unwrap_or(&empty);
    let (requested_global, requested_per_model) = requested_estimates(request, stored);
    let stored_global = stored
        .get(ESTIMATED_OUTPUT_TOKENS_FIELD)
        .and_then(present_value);
    let stored_per_model = stored.get(ESTIMATE_PER_MODEL_FIELD).and_then(present_value);
    let unchanged = same_value(requested_global.as_ref(), stored_global.as_ref())
        && same_value(requested_per_model.as_ref(), stored_per_model.as_ref());
    if unchanged {
        return Ok(());
    }
    Err(AuthLimitInputsError::EstimateAdminOnly { entity })
}

fn estimate_from_metadata(
    metadata: &BTreeMap<String, JsonData>,
    model_name: Option<&str>,
) -> Option<i64> {
    let declares_estimate = metadata.contains_key(ESTIMATED_OUTPUT_TOKENS_FIELD)
        || metadata.contains_key(ESTIMATE_PER_MODEL_FIELD);
    if !declares_estimate {
        return None;
    }
    if let Some(requested) = model_name {
        let per_model = match metadata.get(ESTIMATE_PER_MODEL_FIELD) {
            None | Some(JsonData::Null) => None,
            Some(raw) => validated_estimates(raw),
        };
        if let Some(estimate) = per_model.as_ref().and_then(|limits| limits.get(requested)) {
            return Some(*estimate);
        }
    }
    match metadata.get(ESTIMATED_OUTPUT_TOKENS_FIELD) {
        None | Some(JsonData::Null) => None,
        Some(raw) => validated_estimate(raw),
    }
}

fn validated_estimate(raw: &JsonData) -> Option<i64> {
    match positive_int(raw) {
        Some(estimate) => Some(estimate),
        None => {
            warn_ignored(ESTIMATED_OUTPUT_TOKENS_FIELD);
            None
        }
    }
}

fn validated_estimates(raw: &JsonData) -> Option<BTreeMap<String, i64>> {
    let JsonData::Object(fields) = raw else {
        warn_ignored(ESTIMATE_PER_MODEL_FIELD);
        return None;
    };
    let mut estimates = BTreeMap::new();
    for (model, value) in fields {
        let Some(estimate) = positive_int(value) else {
            warn_ignored(ESTIMATE_PER_MODEL_FIELD);
            return None;
        };
        estimates.insert(model.clone(), estimate);
    }
    Some(estimates)
}

// pydantic's PositiveInt: true and positive whole numbers, text int() reads, everything else fails
fn positive_int(raw: &JsonData) -> Option<i64> {
    let value = match raw {
        JsonData::Bool(flag) => i64::from(*flag),
        JsonData::Number(number) => number_integer(number)?,
        JsonData::String(text) => pydantic_integer(text)?,
        JsonData::Null | JsonData::Array(_) | JsonData::Object(_) => return None,
    };
    (value > 0).then_some(value)
}

fn number_integer(number: &Number) -> Option<i64> {
    if let Some(value) = number.as_i64() {
        return Some(value);
    }
    if let Some(value) = number.as_u64() {
        return i64::try_from(value).ok();
    }
    let float = number.as_f64()?;
    if !float.is_finite() || float.fract().to_bits() != 0.0_f64.to_bits() {
        return None;
    }
    format!("{float:.0}").parse::<i64>().ok()
}

// pydantic's int from text takes a sign, digits with single underscores, and a fraction of zeros only
fn pydantic_integer(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let (integral, fraction) = match trimmed.split_once('.') {
        Some((integral, fraction)) => (integral, Some(fraction)),
        None => (trimmed, None),
    };
    if fraction.is_some_and(|rest| rest.is_empty() || !rest.bytes().all(|byte| byte == b'0')) {
        return None;
    }
    let digits = integral.strip_prefix(['+', '-']).unwrap_or(integral);
    let malformed = digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_');
    if malformed {
        return None;
    }
    integral.replace('_', "").parse::<i64>().ok()
}

fn warn_ignored(field: &'static str) {
    tracing::warn!(
        field,
        "ignoring a malformed output token estimate in metadata"
    );
}

fn requested_estimates(
    request: &OutputTokenEstimateRequest<'_>,
    existing_metadata: &BTreeMap<String, JsonData>,
) -> (Option<JsonData>, Option<JsonData>) {
    let empty = BTreeMap::new();
    let base = if request
        .model_fields_set
        .contains(&OutputTokenEstimateField::Metadata)
    {
        request.metadata.unwrap_or(&empty)
    } else {
        existing_metadata
    };
    let global = match request.estimate {
        Some(estimate) => Some(JsonData::Number(Number::from(estimate))),
        None => base
            .get(ESTIMATED_OUTPUT_TOKENS_FIELD)
            .and_then(present_value),
    };
    let per_model = match request.estimate_per_model {
        Some(estimates) => Some(estimate_map_value(estimates)),
        None => base.get(ESTIMATE_PER_MODEL_FIELD).and_then(present_value),
    };
    (global, per_model)
}

fn estimate_map_value(estimates: &BTreeMap<String, i64>) -> JsonData {
    let fields: BTreeMap<String, JsonData> = estimates
        .iter()
        .map(|(model, estimate)| (model.clone(), JsonData::Number(Number::from(*estimate))))
        .collect();
    JsonData::Object(fields)
}

pub(super) fn present_value(value: &JsonData) -> Option<JsonData> {
    if matches!(value, JsonData::Null) {
        return None;
    }
    Some(value.clone())
}

pub(super) fn same_value(left: Option<&JsonData>, right: Option<&JsonData>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => left == right || numbers_equal(left, right),
        (Some(_), None) | (None, Some(_)) => false,
    }
}

fn numbers_equal(left: &JsonData, right: &JsonData) -> bool {
    let (JsonData::Number(left), JsonData::Number(right)) = (left, right) else {
        return false;
    };
    number_equals(left, right)
}

// Python compares 2000 and 2000.0 equal, so a stored float must not look like a change
fn number_equals(left: &Number, right: &Number) -> bool {
    if let (Some(left), Some(right)) = (left.as_i64(), right.as_i64()) {
        return left == right;
    }
    if let (Some(left), Some(right)) = (left.as_u64(), right.as_u64()) {
        return left == right;
    }
    match (left.as_f64(), right.as_f64()) {
        (Some(left), Some(right)) => {
            match (
                Decimal::from_f64_retain(left),
                Decimal::from_f64_retain(right),
            ) {
                (Some(left), Some(right)) => left == right,
                _ => false,
            }
        }
        _ => false,
    }
}
