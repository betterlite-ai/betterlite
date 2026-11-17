use std::collections::BTreeMap;

use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::usage::AnthropicUsage;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_models::decisions::Json;
use http::header::HeaderMap;
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use super::support::{AUTH_HEADERS, EXPLICIT_HEADERS, header_fields};

pub(crate) fn request_view(request: &OutboundRequest) -> Value {
    let auth = AUTH_HEADERS
        .into_iter()
        .find(|name| request.headers.contains_key(*name))
        .map_or(Value::Null, |name| json!(name));
    json!({
        "method": request.method.as_str(),
        "url": request.url.as_str(),
        "headers": header_fields(&request.headers, Some(EXPLICIT_HEADERS.as_slice())),
        "auth_header": auth,
    })
}

pub(crate) fn pre_call_view(details: &IndexMap<String, Json>) -> Value {
    let Some(Json::Object(additional)) = details.get("additional_args") else {
        return Value::Null;
    };
    let Some(Json::Text(input)) = details.get("input") else {
        return Value::Null;
    };
    let Some(Json::Object(headers)) = additional.get("headers") else {
        return Value::Null;
    };
    let mut fields = BTreeMap::new();
    for name in EXPLICIT_HEADERS {
        if let Some(value) = headers.get(name) {
            fields.insert(name.to_owned(), value.clone());
        }
    }
    let auth = AUTH_HEADERS
        .into_iter()
        .find(|name| headers.contains_key(*name))
        .map_or(Value::Null, |name| json!(name));
    json!({
        "input": input,
        "api_base": additional.get("api_base").cloned().unwrap_or(Json::Null),
        "headers": fields,
        "auth_header": auth,
        "complete_input_dict": additional.get("complete_input_dict").cloned().unwrap_or(Json::Null),
    })
}

pub(crate) fn response_view(envelope: &ResponseEnvelope) -> Value {
    let choice = envelope.choices.first();
    let mut fields = Map::new();
    fields.insert("id".to_owned(), json!(envelope.id));
    fields.insert("object".to_owned(), json!(envelope.object));
    fields.insert("created".to_owned(), json!(envelope.created));
    fields.insert("model".to_owned(), json!(envelope.model));
    fields.insert(
        "content".to_owned(),
        json!(choice.and_then(|chosen| chosen.message.content.clone())),
    );
    fields.insert(
        "finish_reason".to_owned(),
        json!(choice.and_then(|chosen| chosen.finish_reason.clone())),
    );
    fields.insert(
        "usage".to_owned(),
        envelope.usage.as_ref().map_or(Value::Null, usage_view),
    );
    Value::Object(fields)
}

fn usage_view(usage: &AnthropicUsage) -> Value {
    let prompt = &usage.prompt_tokens_details;
    let completion = &usage.completion_tokens_details;
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_text_tokens": prompt.text,
        "cached_tokens": prompt.cached,
        "cache_creation_tokens": prompt.cache_creation,
        "cache_write_tokens": prompt.cache_write,
        "completion_text_tokens": completion.text,
        "reasoning_tokens": completion.reasoning,
        "accepted_prediction_tokens": completion.accepted_prediction,
    })
}

pub(crate) fn auth_of(headers: &HeaderMap, expected: Option<&str>) -> Value {
    for name in AUTH_HEADERS {
        if let Some(value) = headers.get(name) {
            let matches = value.to_str().ok() == expected;
            return json!({"name": name, "matches": matches});
        }
    }
    json!({"name": Value::Null, "matches": expected.is_none()})
}
