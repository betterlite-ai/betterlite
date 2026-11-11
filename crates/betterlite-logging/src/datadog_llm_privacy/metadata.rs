use betterlite_models::decisions::Json;

use super::guardrail_redact::redacted_records;
use super::json_number::number as json_number;
use super::latency::latency_metrics;
use super::spend::spend_metrics;
use super::stream::is_streamed_request;
use super::{RecordEncodingError, record_object};
use crate::datadog_llm_obs::constants::METADATA_PROMPT_FIELDS;
use crate::datadog_llm_obs::values::Object;
use crate::fields::payload::StandardLoggingPayload;

// LiteLLM's _get_dd_llm_obs_payload_metadata: the fixed span fields, the request metadata, router fields, cost tags
pub fn span_metadata(
    payload: &StandardLoggingPayload,
    router_fields: &Object,
    cost_tags: &[String],
    redact_prompt_text: bool,
) -> Result<Object, RecordEncodingError> {
    let mut metadata = fixed_fields(payload, redact_prompt_text)?;
    for (field, value) in record_object("metadata", &payload.metadata)? {
        if redact_prompt_text && METADATA_PROMPT_FIELDS.contains(&field.as_str()) {
            continue;
        }
        metadata.insert(field, value);
    }
    for (field, value) in router_fields {
        metadata.insert(field.clone(), value.clone());
    }
    if !cost_tags.is_empty() {
        metadata.insert("_dd".to_owned(), Json::Object(dd_fields(cost_tags)));
    }
    Ok(metadata)
}

fn fixed_fields(
    payload: &StandardLoggingPayload,
    redact_prompt_text: bool,
) -> Result<Object, RecordEncodingError> {
    let unknown = || Json::Text("unknown".to_owned());
    let mut metadata = Object::new();
    metadata.insert("model_name".to_owned(), Json::Text(payload.model.clone()));
    let provider = payload.custom_llm_provider.clone();
    metadata.insert(
        "model_provider".to_owned(),
        provider.map_or_else(unknown, Json::Text),
    );
    metadata.insert("id".to_owned(), Json::Text(payload.id.clone()));
    metadata.insert("trace_id".to_owned(), Json::Text(payload.trace_id.clone()));
    metadata.insert(
        "cache_hit".to_owned(),
        payload.cache_hit.map_or_else(unknown, Json::Bool),
    );
    let cache_key = payload.cache_key.clone();
    metadata.insert(
        "cache_key".to_owned(),
        cache_key.map_or_else(unknown, Json::Text),
    );
    let cost = json_number(payload.saved_cache_cost);
    metadata.insert("saved_cache_cost".to_owned(), cost);
    let guardrails = guardrail_value(payload, redact_prompt_text)?;
    metadata.insert("guardrail_information".to_owned(), guardrails);
    let streamed = Json::Bool(is_streamed_request(payload));
    metadata.insert("is_streamed_request".to_owned(), streamed);
    let latency = record_object("latency_metrics", &latency_metrics(payload))?;
    metadata.insert("latency_metrics".to_owned(), Json::Object(latency));
    let spend = record_object("spend_metrics", &spend_metrics(payload))?;
    metadata.insert("spend_metrics".to_owned(), Json::Object(spend));
    Ok(metadata)
}

fn guardrail_value(
    payload: &StandardLoggingPayload,
    redact_prompt_text: bool,
) -> Result<Json, RecordEncodingError> {
    let Some(records) = payload.guardrail_information.as_deref() else {
        return Ok(Json::Null);
    };
    if redact_prompt_text {
        let redacted = redacted_records(records)?;
        return Ok(Json::List(redacted.into_iter().map(Json::Object).collect()));
    }
    let mut encoded = Vec::with_capacity(records.len());
    for record in records {
        encoded.push(Json::Object(record_object("guardrail", record)?));
    }
    Ok(Json::List(encoded))
}

fn dd_fields(cost_tags: &[String]) -> Object {
    let mut tags = Vec::with_capacity(cost_tags.len());
    for tag in cost_tags {
        tags.push(Json::Text(tag.clone()));
    }
    let mut fields = Object::new();
    fields.insert("cost_tags".to_owned(), Json::List(tags));
    fields
}

mod tests;
