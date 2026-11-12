use std::collections::BTreeMap;

use betterlite_models::decisions::Extra;
use betterlite_models::decisions::Json;
use betterlite_vertex::multimodal_embedding_records::error::MultimodalEmbeddingError;
use betterlite_vertex::multimodal_embedding_records::request::map_openai_params;
use betterlite_vertex::multimodal_embedding_records::request::transform_request;
use betterlite_vertex::multimodal_embedding_records::request::validate_environment;
use betterlite_vertex::multimodal_embedding_records::response::MultimodalEmbeddingAnswer;
use betterlite_vertex::multimodal_embedding_records::response::transform_response;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    headers: Vec<HeaderCall>,
    mappings: Vec<MappingCall>,
    requests: Vec<RequestCall>,
    replies: Vec<ReplyCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeaderCall {
    caller: BTreeMap<String, String>,
    api_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MappingCall {
    options: Extra,
    optional: Extra,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestCall {
    input: Json,
    optional: Extra,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyCall {
    model: String,
    status: u16,
    response: Json,
    input: Json,
    optional: Extra,
}

pub(super) fn run(input: &Input) -> Value {
    json!({
        "headers": header_entries(&input.headers),
        "mappings": mapping_entries(&input.mappings),
        "requests": request_entries(&input.requests),
        "replies": reply_entries(&input.replies),
    })
}

fn header_entries(calls: &[HeaderCall]) -> Vec<Value> {
    let mut entries = Vec::with_capacity(calls.len());
    for call in calls {
        entries.push(json!({
            "headers": validate_environment(call.caller.clone(), &call.api_key),
        }));
    }
    entries
}

fn mapping_entries(calls: &[MappingCall]) -> Vec<Value> {
    let mut entries = Vec::with_capacity(calls.len());
    for call in calls {
        entries.push(json!({
            "optional": map_openai_params(&call.options, call.optional.clone()),
        }));
    }
    entries
}

fn request_entries(calls: &[RequestCall]) -> Vec<Value> {
    let mut entries = Vec::with_capacity(calls.len());
    for call in calls {
        entries.push(
            transform_request(&call.input, &call.optional)
                .map_or_else(|_| json!({"error": true}), |body| json!(body)),
        );
    }
    entries
}

fn reply_entries(calls: &[ReplyCall]) -> Vec<Value> {
    let mut entries = Vec::with_capacity(calls.len());
    for call in calls {
        entries.push(reply_entry(call));
    }
    entries
}

fn reply_entry(call: &ReplyCall) -> Value {
    let Ok(request) = transform_request(&call.input, &call.optional) else {
        return json!({"error": true});
    };
    let Ok(body) = serde_json::to_vec(&call.response) else {
        return json!({"error": true});
    };
    match transform_response(call.status, &body, &request, &call.model) {
        Ok(answer) => answer_value(&answer),
        Err(MultimodalEmbeddingError::MissingPredictions) => json!({
            "error": {
                "status": 500,
                "type": "internal_server_error",
                "param": null,
                "code": null,
            },
        }),
        Err(_) => json!({"error": true}),
    }
}

fn answer_value(answer: &MultimodalEmbeddingAnswer) -> Value {
    json!({
        "model": answer.response.model,
        "data": answer.response.data,
        "usage": {
            "prompt_tokens": answer.response.usage.prompt_tokens,
            "completion_tokens": answer.response.usage.completion_tokens,
            "total_tokens": answer.response.usage.total_tokens,
            "prompt_tokens_details": {
                "character_count": answer.details.character_count,
                "image_count": answer.details.image_count,
                "video_length_seconds": answer.details.video_length_seconds.to_string(),
            },
        },
    })
}
