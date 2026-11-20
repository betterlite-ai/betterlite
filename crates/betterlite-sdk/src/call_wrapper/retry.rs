use betterlite_error::error::ErrorClass;
use betterlite_error::error::retry_policy::{RetryFamily, RetryPolicy};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::{CallError, class_status, field_error};
use super::fields::{field, object_field};
use crate::call_setup::RequestMap;

// litellm.num_retries and litellm.num_retries_per_request, the module state the decorator retries from
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WrapperRetryDefaults {
    pub num_retries: Option<i64>,
    pub num_retries_per_request: Option<i64>,
}

// The sync path replaces the count with the policy's answer; the async path keeps it unless the policy names one
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryMode {
    Sync,
    Async,
}

// Newer endpoints carry litellm_metadata, older ones carry metadata, and only one of them holds request_retry_count
#[must_use]
pub fn metadata_key(request: &RequestMap) -> &'static str {
    if request.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    }
}

#[must_use]
pub fn max_retries_per_request_hit(request: &RequestMap, limit: Option<i64>) -> bool {
    let Some(limit_value) = limit else {
        return false;
    };
    let Some(Json::Object(metadata)) = request.get(metadata_key(request)) else {
        return false;
    };
    let Some(Json::Number(retry_count)) = metadata.get("request_retry_count") else {
        return false;
    };
    retry_count
        .as_i64()
        .is_some_and(|count| 0 < count && limit_value <= count)
}

// The router keeps model_group in the request's own metadata, never in litellm_metadata
pub fn is_router_call(request: &RequestMap) -> Result<bool, CallError> {
    let Some(metadata) = object_field(request, "metadata")? else {
        return Ok(false);
    };
    Ok(metadata.contains_key("model_group"))
}

// Only a plain Exception, BudgetExceededError and GuardrailRaisedException sit outside openai.APIError
#[must_use]
pub const fn is_api_error(class: &ErrorClass) -> bool {
    !matches!(
        class,
        ErrorClass::Unclassified(_)
            | ErrorClass::BudgetExceeded(_)
            | ErrorClass::GuardrailRaised(_)
    )
}

// RateLimitError takes the exponential branch before the generic APIError one
#[must_use]
pub const fn retry_strategy(class: &ErrorClass) -> Option<&'static str> {
    match class {
        ErrorClass::RateLimit(_) => Some("exponential_backoff_retry"),
        ErrorClass::Authentication(_)
        | ErrorClass::NotFound(_)
        | ErrorClass::PermissionDenied(_)
        | ErrorClass::BadRequest(_)
        | ErrorClass::UnprocessableEntity(_)
        | ErrorClass::Timeout(_)
        | ErrorClass::ContextWindowExceeded(_)
        | ErrorClass::ContentPolicyViolation(_)
        | ErrorClass::ServiceUnavailable(_)
        | ErrorClass::BadGateway(_)
        | ErrorClass::InternalServer(_)
        | ErrorClass::Api(_)
        | ErrorClass::ApiConnection(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::UnsupportedParams(_)
        | ErrorClass::MidStreamFallback(_) => Some("constant_retry"),
        ErrorClass::BudgetExceeded(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::Unclassified(_) => None,
    }
}

fn policy_from(request: &RequestMap) -> Result<Option<RetryPolicy>, CallError> {
    let fields = match request.get("retry_policy") {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Object(fields)) => fields,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(field_error("retry_policy", "an object of retry counts"));
        }
    };
    if fields.is_empty() {
        return Ok(None);
    }
    let value = serde_json::to_value(Json::Object(fields.clone()))
        .map_err(|_error| field_error("retry_policy", "an object of retry counts"))?;
    let policy = serde_json::from_value::<RetryPolicy>(value)
        .map_err(|_error| field_error("retry_policy", "an object of retry counts"))?;
    Ok(Some(policy))
}

pub fn num_retries(
    request: &mut RequestMap,
    default: Option<i64>,
    error: &CallError,
    mode: RetryMode,
) -> Result<Option<i64>, CallError> {
    let from_request: Option<i64> = field(request, "num_retries")?;
    let mut num_retries = from_request.or(default);
    if let Some(policy) = policy_from(request)? {
        let family = RetryFamily::from(&error.class);
        let from_policy = policy.retries_for(family, class_status(&error.class));
        request.insert("retry_policy".to_owned(), Json::Object(IndexMap::new()));
        if mode == RetryMode::Sync || from_policy.is_some() {
            num_retries = from_policy;
        }
    }
    Ok(num_retries)
}

pub fn context_window_model(
    request: &RequestMap,
    model: Option<&str>,
) -> Result<Option<String>, CallError> {
    let Some(model_name) = model else {
        return Ok(None);
    };
    let Some(mapping) = object_field(request, "context_window_fallback_dict")? else {
        return Ok(None);
    };
    if mapping.is_empty() {
        return Ok(None);
    }
    match mapping.get(model_name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(mapped)) => Ok(Some(mapped.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => Err(
            field_error("context_window_fallback_dict", "an object of model names"),
        ),
    }
}
