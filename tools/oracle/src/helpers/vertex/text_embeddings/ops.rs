use betterlite_models::decisions::Extra;
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingInput;
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
        "mappings": mappings(&input.mappings),
        "requests": requests(&input.requests),
        "replies": replies(&input.replies),
    })
}

fn mappings(calls: &[Mapping]) -> Vec<Value> {
    calls
        .iter()
        .map(|call| {
            let (optional, kwargs) =
                map_openai_params(&call.options, call.optional.clone(), call.kwargs.clone());
            // LiteLLM maps the auth aliases on the dict the OpenAI mapping already changed
            let auth = map_special_auth_params(&call.options, optional.clone());
            json!({
                "optional": optional,
                "kwargs": kwargs,
                "auth": auth,
            })
        })
        .collect()
}

fn requests(calls: &[Request]) -> Vec<Value> {
    calls
        .iter()
        .map(|call| {
            let built = transform_request(
                &call.input,
                &call.optional,
                &call.model,
                call.litellm_params.as_ref(),
            );
            match built {
                Ok(request) => json!(request),
                Err(_error) => json!({"error": true}),
            }
        })
        .collect()
}

fn replies(calls: &[Reply]) -> Vec<Value> {
    calls
        .iter()
        .map(
            |call| match transform_response(&call.response, &call.model) {
                Ok(answer) => json!({
                    "model": answer.model,
                    "object": answer.object,
                    "data": answer.data,
                    "usage": {
                        "prompt_tokens": answer.usage.prompt_tokens,
                        "completion_tokens": answer.usage.completion_tokens,
                        "total_tokens": answer.usage.total_tokens,
                    },
                }),
                Err(_error) => json!({"error": true}),
            },
        )
        .collect()
}
