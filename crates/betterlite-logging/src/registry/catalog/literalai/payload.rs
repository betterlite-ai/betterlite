use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::fields::payload::StandardLoggingPayload;
use crate::literal_ai::literal_ai_logging::payload::{
    CallbackKwargs, LitellmParams, StandardLoggingPayload as LiteralLoggingPayload,
};

// Literal AI keeps its own StandardLoggingPayload, so each field is copied from the standard payload's field with the same name
fn logging_payload(payload: &StandardLoggingPayload) -> LiteralLoggingPayload {
    LiteralLoggingPayload {
        metadata: metadata_fields(payload),
        model_parameters: payload.model_parameters.clone(),
        messages: payload.messages.clone(),
        response: payload.response.clone(),
        error_str: payload.error_str.clone(),
        prompt_tokens: payload.prompt_tokens,
        completion_tokens: payload.completion_tokens,
        total_tokens: payload.total_tokens,
    }
}

// the standard metadata is a typed struct, and literalai takes the object LiteLLM hands it in the logging payload
fn metadata_fields(payload: &StandardLoggingPayload) -> IndexMap<String, Json> {
    match serde_json::to_value(&payload.metadata).and_then(serde_json::from_value::<Json>) {
        Ok(Json::Object(fields)) => fields,
        Ok(value) => {
            tracing::warn!(?value, "the literalai callback metadata is not an object");
            IndexMap::new()
        }
        Err(error) => {
            tracing::warn!(%error, "the literalai callback metadata could not be encoded");
            IndexMap::new()
        }
    }
}

const USER_AGENT_TAG: &str = "User-Agent: ";

// _get_request_tags appends its User-Agent tags after the metadata tags, and betterlite has no extra_spend_tag_headers to append more
fn metadata_tags(payload: &StandardLoggingPayload) -> Vec<Json> {
    let appended = payload
        .request_tags
        .iter()
        .rev()
        .take_while(|tag| matches!(tag, Json::Text(text) if text.starts_with(USER_AGENT_TAG)))
        .count();
    let logged = payload.request_tags.len().saturating_sub(appended);
    payload.request_tags.iter().take(logged).cloned().collect()
}

// a request's own metadata rides nested in the standard payload, and literalai reads step_id and the literalai_* keys flat
fn request_metadata(payload: &StandardLoggingPayload) -> IndexMap<String, Json> {
    let mut fields = metadata_fields(payload);
    if let Some(Json::Object(requester)) = payload.metadata.requester_metadata.as_ref() {
        fields.extend(requester.clone());
    }
    // the proxy merges the body, key, team and project tags into litellm_params.metadata.tags, which literalai reads before literalai_tags
    if !fields.contains_key("tags") {
        let tags = metadata_tags(payload);
        if !tags.is_empty() {
            fields.insert("tags".to_owned(), Json::List(tags));
        }
    }
    fields
}

pub(super) fn callback_kwargs(payload: &StandardLoggingPayload) -> CallbackKwargs {
    CallbackKwargs {
        standard_logging_object: Some(logging_payload(payload)),
        litellm_params: Some(LitellmParams {
            metadata: Some(request_metadata(payload)),
        }),
        model: Some(payload.model.clone()),
        custom_llm_provider: payload.custom_llm_provider.clone(),
    }
}

// the logging payload carries epoch seconds with microseconds, and the step's duration is measured in microseconds
pub(super) fn utc_at(seconds: Decimal) -> Option<DateTime<Utc>> {
    seconds
        .checked_mul(Decimal::from(1_000_000_i64))
        .and_then(|micros| micros.round().to_i64())
        .and_then(DateTime::<Utc>::from_timestamp_micros)
}
