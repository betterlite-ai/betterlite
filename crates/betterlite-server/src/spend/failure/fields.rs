use betterlite_auth::auxiliary_routes::INFO_ROUTES;
use betterlite_auth::route_match::classifier::is_llm_api_route;
use betterlite_auth::route_match::templates::RouteValue;
use betterlite_config::spend_storage::SpendUpdates;
use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_logging::langfuse_projections::steering::truthiness;
use betterlite_logging::spend_bodies::session::SpendTimestamp;
use betterlite_models::decisions::Json;
use betterlite_models::guardrail_pricing::GuardrailPricingError;
use betterlite_models::guardrail_pricing::entries::{
    GuardrailInformation, guardrail_information_cost,
};
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
use chrono::{DateTime, NaiveDateTime};
use rust_decimal::Decimal;
use thiserror::Error;

use super::metadata::{
    COMBINED_USAGE_OBJECT, COMPLETION_START_TIME, GUARDRAIL_INFORMATION, LITELLM_METADATA,
    LITELLM_PARAMS, LITELLM_TRACE_ID, METADATA, RESPONSE_COST, STANDARD_LOGGING_OBJECT, START_TIME,
    TAGS, USER_API_KEY_PREFIX, as_object, object_mut, take_litellm_params,
};
use crate::spend::payload::CompletionStart;

const NAIVE_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.f";
const SPACED_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.f";
const REQUEST_TAGS: &str = "request_tags";
pub(crate) const TRACKABLE_ANONYMOUS_CALL_TYPES: [&str; 5] = [
    "pass_through_endpoint",
    "llm_passthrough_route",
    "allm_passthrough_route",
    "call_mcp_tool",
    "aretrieve_batch",
];

#[derive(Debug, Error)]
pub enum FailureCostError {
    #[error("response_cost must be a number")]
    ResponseCost,
    #[error("the response cost is not a number")]
    Decimal {
        #[source]
        source: DecimalLexemeError,
    },
    #[error("the guardrail information could not be read")]
    Guardrail {
        #[source]
        source: serde_json::Error,
    },
    #[error("the guardrail cost could not be computed")]
    GuardrailPricing {
        #[source]
        source: GuardrailPricingError,
    },
    #[error("the recovered failure cost is too large")]
    Overflow,
}

// The logging object's fields the failure payload lifts; the wiring builds it from B1708's Logging.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallLog {
    pub standard_logging_object: Option<Json>,
    pub litellm_trace_id: Option<String>,
    pub start_time: Option<SpendTimestamp>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrackingIdentity<'facts> {
    pub hashed_user_api_key: Option<&'facts str>,
    pub user_id: Option<&'facts str>,
    pub team_id: Option<&'facts str>,
    pub end_user_id: Option<&'facts str>,
    pub agent_id: Option<&'facts str>,
}

pub(crate) fn lift_call_log(request_data: &mut JsonObject, call_log: &CallLog) {
    let standard_missing = request_data
        .get(STANDARD_LOGGING_OBJECT)
        .is_none_or(|value| !truthiness(value));
    if standard_missing {
        let lifted = call_log
            .standard_logging_object
            .clone()
            .unwrap_or(Json::Null);
        request_data.insert(STANDARD_LOGGING_OBJECT.to_owned(), lifted);
    }
    let trace_missing = request_data
        .get(LITELLM_TRACE_ID)
        .is_none_or(|value| matches!(value, Json::Null));
    if trace_missing {
        request_data.insert(
            LITELLM_TRACE_ID.to_owned(),
            call_log
                .litellm_trace_id
                .clone()
                .map_or(Json::Null, Json::Text),
        );
    }
}

#[must_use]
pub(crate) fn failure_start_time(
    request_data: &JsonObject,
    call_log: Option<&CallLog>,
    now: SpendTimestamp,
) -> SpendTimestamp {
    request_data
        .get(START_TIME)
        .and_then(datetime_value)
        .or_else(|| call_log.and_then(|log| log.start_time))
        .unwrap_or(now)
}

#[must_use]
pub(crate) fn completion_start(request_data: &JsonObject) -> CompletionStart {
    match request_data.get(COMPLETION_START_TIME) {
        None => CompletionStart::Absent,
        Some(Json::Null) => CompletionStart::Null,
        Some(value) => datetime_value(value).map_or(CompletionStart::Absent, CompletionStart::At),
    }
}

// max(float(response_cost), 0.0) only when combined_usage_object is a typed Usage, plus guardrail charges.
pub(crate) fn failure_response_cost(
    request_data: &JsonObject,
    metadata: Option<&JsonObject>,
) -> Result<Decimal, FailureCostError> {
    let stream = recovered_stream_cost(request_data)?;
    let guardrails = guardrail_cost(metadata)?;
    stream
        .checked_add(guardrails)
        .ok_or(FailureCostError::Overflow)
}

fn recovered_stream_cost(request_data: &JsonObject) -> Result<Decimal, FailureCostError> {
    let Some(Json::Object(usage)) = request_data.get(COMBINED_USAGE_OBJECT) else {
        return Ok(Decimal::ZERO);
    };
    if usage_from_mapping(usage).is_err() {
        return Ok(Decimal::ZERO);
    }
    let cost = match request_data.get(RESPONSE_COST) {
        None | Some(Json::Null) => return Ok(Decimal::ZERO),
        Some(Json::Number(number)) => parse_decimal_lexeme(&number.to_string())
            .map_err(|source| FailureCostError::Decimal { source })?,
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(FailureCostError::ResponseCost);
        }
    };
    Ok(cost.max(Decimal::ZERO))
}

fn guardrail_cost(metadata: Option<&JsonObject>) -> Result<Decimal, FailureCostError> {
    let Some(value) = metadata.and_then(|fields| fields.get(GUARDRAIL_INFORMATION)) else {
        return Ok(Decimal::ZERO);
    };
    let encoded =
        serde_json::to_string(value).map_err(|source| FailureCostError::Guardrail { source })?;
    let information: GuardrailInformation =
        serde_json::from_str(&encoded).map_err(|source| FailureCostError::Guardrail { source })?;
    guardrail_information_cost(&information)
        .map_err(|source| FailureCostError::GuardrailPricing { source })
}

// _write_spend_metadata_to_kwargs: only non-null key fields and tags fill a null in an existing bucket.
pub fn write_spend_metadata_to_kwargs(kwargs: &mut JsonObject, metadata: &JsonObject) {
    let patch: Vec<(String, Json)> = metadata
        .iter()
        .filter(|(key, value)| {
            !matches!(value, Json::Null)
                && (key.starts_with(USER_API_KEY_PREFIX) || key.as_str() == TAGS)
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if patch.is_empty() {
        return;
    }
    let mut params = take_litellm_params(kwargs);
    for bucket_name in [LITELLM_METADATA, METADATA] {
        let Some(bucket) = params.get_mut(bucket_name).and_then(object_mut) else {
            continue;
        };
        for (key, value) in &patch {
            if bucket
                .get(key)
                .is_none_or(|current| matches!(current, Json::Null))
            {
                bucket.insert(key.clone(), value.clone());
            }
        }
    }
    kwargs.insert(LITELLM_PARAMS.to_owned(), Json::Object(params));
}

// _get_request_tags_for_cost_tracking: a list-valued standard payload wins, even an empty one.
#[must_use]
pub fn request_tags_for_cost_tracking<'json>(
    standard_logging_object: Option<&'json Json>,
    metadata: Option<&'json Json>,
) -> Option<&'json Json> {
    if let Some(tags) = standard_logging_object
        .and_then(as_object)
        .and_then(|fields| fields.get(REQUEST_TAGS))
        && matches!(tags, Json::List(_))
    {
        return Some(tags);
    }
    metadata
        .and_then(as_object)
        .and_then(|fields| fields.get(TAGS))
        .filter(|tags| matches!(tags, Json::List(_)))
}

// _should_track_cost_callback: the shared exact-true spend switch runs before any identity counts.
#[must_use]
pub fn should_track_cost_callback(
    spend_updates: SpendUpdates,
    identity: &TrackingIdentity<'_>,
    call_type: Option<&str>,
) -> bool {
    if spend_updates == SpendUpdates::Disabled {
        return false;
    }
    if identity.hashed_user_api_key.is_some()
        || identity.user_id.is_some()
        || identity.team_id.is_some()
        || identity.end_user_id.is_some()
        || identity.agent_id.is_some()
    {
        return true;
    }
    call_type.is_some_and(|name| TRACKABLE_ANONYMOUS_CALL_TYPES.contains(&name))
}

// _metadata_keys: a mapping's keys sorted, and nothing for any other value.
#[must_use]
pub fn metadata_keys(metadata: Option<&Json>) -> Vec<String> {
    let Some(Json::Object(fields)) = metadata else {
        return Vec::new();
    };
    let mut keys: Vec<String> = fields.keys().cloned().collect();
    keys.sort_unstable();
    keys
}

#[must_use]
pub(crate) fn eligible_route(route: Option<&str>) -> bool {
    match route {
        None => true,
        Some(route) => is_llm_api_route(&RouteValue::Text(route)) || INFO_ROUTES.contains(&route),
    }
}

fn datetime_value(value: &Json) -> Option<SpendTimestamp> {
    let Json::Text(text) = value else {
        return None;
    };
    if let Ok(aware) = DateTime::parse_from_rfc3339(text) {
        return Some(SpendTimestamp::Aware(aware));
    }
    NaiveDateTime::parse_from_str(text, NAIVE_FORMAT)
        .or_else(|_| NaiveDateTime::parse_from_str(text, SPACED_FORMAT))
        .ok()
        .map(SpendTimestamp::Naive)
}
