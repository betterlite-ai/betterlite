use betterlite_models::decisions::Extra;
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_vertex::vertex_text_embeddings::request::map_openai_params;
use betterlite_vertex::vertex_text_embeddings::request::map_special_auth_params;
use betterlite_vertex::vertex_text_embeddings::request::transform_request;
use betterlite_vertex::vertex_text_embeddings::response::transform_response;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    mappings: Vec<Mapping>,
    #[serde(default)]
    requests: Vec<Request>,
    #[serde(default)]
    replies: Vec<Reply>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mapping {
    #[serde(default)]
    options: Extra,
    #[serde(default)]
    optional: Extra,
    #[serde(default)]
    kwargs: Extra,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    model: String,
    input: EmbeddingInput,
    optional: Json,
    litellm_params: Option<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    model: String,
    response: Json,
}

pub(super) fn run(input: &Input) -> Value {
    json!({
        "mappings": entries(&input.mappings, mapping),
        "requests": fallible_entries(&input.requests, request),
        "replies": fallible_entries(&input.replies, reply),
    })
}

fn entries<Call>(calls: &[Call], convert: fn(&Call) -> Value) -> Vec<Value> {
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        values.push(convert(call));
    }
    values
}

fn fallible_entries<Call>(calls: &[Call], convert: fn(&Call) -> Option<Value>) -> Vec<Value> {
    calls
        .iter()
        .map(|call| convert(call).unwrap_or_else(|| json!({"error": true})))
        .collect()
}

fn mapping(call: &Mapping) -> Value {
    let (optional, kwargs) =
        map_openai_params(&call.options, call.optional.clone(), call.kwargs.clone());
    // LiteLLM maps the auth aliases onto the same dict it returns as the OpenAI mapping
    let mapped = map_special_auth_params(&call.options, optional);
    let auth = mapped.clone();
    json!({"optional": mapped, "kwargs": kwargs, "auth": auth})
}

fn request(call: &Request) -> Option<Value> {
    transform_request(
        &call.input,
        &call.optional,
        &call.model,
        call.litellm_params.as_ref(),
    )
    .ok()
    .map(|request| json!(request))
}

fn reply(call: &Reply) -> Option<Value> {
    transform_response(&call.response, &call.model)
        .ok()
        .map(|answer| embedding_value(&answer))
}

fn embedding_value(answer: &EmbeddingResponse) -> Value {
    json!({
        "model": answer.model,
        "object": answer.object,
        "data": answer.data,
        "usage": {
            "prompt_tokens": answer.usage.prompt_tokens,
            "completion_tokens": answer.usage.completion_tokens,
            "total_tokens": answer.usage.total_tokens,
        },
    })
}
