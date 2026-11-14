use betterlite_provider::api::vector_store_contracts::config::{Headers, Params};
use betterlite_rag::s3_vectors_search::config::{
    RegionEnvironment, complete_url, query_embedding_model,
};
use betterlite_rag::s3_vectors_search::request::{QueryInput, build_query_request};
use betterlite_rag::s3_vectors_search::response::decode_s3_vectors_search;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "call", content = "input", rename_all = "snake_case")]
enum Call {
    Request(RequestCall),
    Response(ResponseCall),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestCall {
    vector_store_id: String,
    query: String,
    vector: Vec<f64>,
    #[serde(default)]
    max_num_results: Option<i64>,
    litellm_params: Params,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseCall {
    status: u16,
    headers: Headers,
    body: String,
    #[serde(default)]
    query: Option<String>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let outcomes = input.calls.iter().map(|call| {
            let value = match call {
                Call::Request(request) => request_value(request),
                Call::Response(response) => response_value(response),
            };
            value.map_or_else(
                || json!({"ok": false}),
                |value| json!({"ok": true, "value": value}),
            )
        });
        Ok(Value::Array(outcomes.collect()))
    }
}

fn request_value(call: &RequestCall) -> Option<Value> {
    let api_base = complete_url(&call.litellm_params, RegionEnvironment::default()).ok()?;
    let request = build_query_request(&QueryInput {
        vector_store_id: &call.vector_store_id,
        query_text: &call.query,
        query_vector: &call.vector,
        max_num_results: call.max_num_results,
        api_base: &api_base,
        litellm_params: &call.litellm_params,
    })
    .ok()?;
    Some(json!({
        "api_base": api_base,
        "url": request.url,
        "body": request.body,
        "embedding_model": query_embedding_model(&call.litellm_params).ok()?,
        "query": request.query_text,
    }))
}

// LiteLLM sets score to None on a hit without a distance, and the record leaves the key out
fn response_value(call: &ResponseCall) -> Option<Value> {
    let response = decode_s3_vectors_search(
        call.status,
        &call.headers,
        call.body.as_bytes(),
        call.query.as_deref(),
    )
    .ok()?;
    let mut value = serde_json::to_value(response).ok()?;
    let results = value.get_mut("data")?.as_array_mut()?;
    for result in results {
        result
            .as_object_mut()?
            .entry("score")
            .or_insert(Value::Null);
    }
    Some(value)
}
