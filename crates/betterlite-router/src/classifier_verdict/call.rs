use betterlite_logging::logging_privacy::PrivacyContext;
use betterlite_logging::logging_privacy::request::{Flag, Metadata, Params, PrivacyRequest};
use betterlite_logging::spend_bodies::support::Fields;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use super::error::ClassifierVerdictError;

pub type RequestKwargs = IndexMap<String, Json>;

#[must_use]
pub fn parent_session_kwargs(request_kwargs: Option<&RequestKwargs>) -> RequestKwargs {
    let mut session = RequestKwargs::new();
    let Some(kwargs) = request_kwargs else {
        return session;
    };
    for key in ["litellm_session_id", "litellm_trace_id"] {
        if let Some(value) = kwargs.get(key).filter(|value| !matches!(value, Json::Null)) {
            session.insert(key.to_owned(), value.clone());
        }
    }
    session
}

#[must_use]
pub fn response_cost_or_none(hidden_params: Option<&IndexMap<String, Json>>) -> Option<Decimal> {
    let Json::Number(number) = hidden_params?.get("response_cost")? else {
        return None;
    };
    number.as_f64().and_then(Decimal::from_f64)
}

pub fn effective_logging_off(
    request_kwargs: Option<&RequestKwargs>,
) -> Result<Option<bool>, ClassifierVerdictError> {
    let Some(dynamic) = dynamic_params(request_kwargs)? else {
        return Ok(None);
    };
    let Some(Json::Bool(value)) = dynamic.get("turn_off_message_logging") else {
        return Ok(None);
    };
    Ok(Some(*value))
}

pub(super) fn dynamic_params(
    request_kwargs: Option<&RequestKwargs>,
) -> Result<Option<Fields>, ClassifierVerdictError> {
    let Some(kwargs) = request_kwargs else {
        return Ok(None);
    };
    let request = privacy_request(kwargs)?;
    let dynamic = request.dynamic_params(PrivacyContext::default())?;
    Ok(Some(dynamic))
}

fn privacy_request(kwargs: &RequestKwargs) -> Result<PrivacyRequest, ClassifierVerdictError> {
    Ok(PrivacyRequest {
        direct: Metadata {
            turn_off_message_logging: kwargs.get("turn_off_message_logging").and_then(flag),
            redact_user_api_key_info: None,
            headers: IndexMap::new(),
            callback_params: IndexMap::new(),
        },
        litellm_params: nested_params(kwargs)?,
        litellm_metadata: metadata_slot(kwargs, "litellm_metadata")?,
        metadata: metadata_slot(kwargs, "metadata")?,
    })
}

fn nested_params(kwargs: &RequestKwargs) -> Result<Option<Params>, ClassifierVerdictError> {
    let Some(Json::Object(params)) = kwargs.get("litellm_params") else {
        return Ok(None);
    };
    let metadata = metadata_slot(params, "metadata")?;
    Ok(Some(Params { metadata }))
}

fn metadata_slot(
    kwargs: &RequestKwargs,
    key: &str,
) -> Result<Option<Metadata>, ClassifierVerdictError> {
    match kwargs.get(key) {
        Some(value @ Json::Object(_)) => Ok(Some(metadata_from(value)?)),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => Ok(None),
    }
}

fn metadata_from(value: &Json) -> Result<Metadata, ClassifierVerdictError> {
    serde_json::from_value(
        serde_json::to_value(value)
            .map_err(|source| ClassifierVerdictError::RequestKwargs { source })?,
    )
    .map_err(|source| ClassifierVerdictError::RequestKwargs { source })
}

fn flag(value: &Json) -> Option<Flag> {
    match value {
        Json::Bool(flag) => Some(Flag::Boolean(*flag)),
        Json::Text(text) => Some(Flag::Text(text.clone())),
        Json::Null | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}
