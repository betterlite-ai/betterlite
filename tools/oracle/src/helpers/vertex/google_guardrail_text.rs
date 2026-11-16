mod tests;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_vertex::google_genai_guardrail::callbacks::merged_request_data;
use betterlite_vertex::google_genai_guardrail::records::{
    request_texts, response_texts, write_texts,
};
use betterlite_vertex::google_genai_guardrail::sse::{
    accumulated_stream_text, decode_google_guardrail_sse,
};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    Request(Box<RequestCall>),
    Response(Box<ResponseCall>),
    Stream(Box<StreamCall>),
    Sse(Box<SseCall>),
}

#[derive(Debug, Deserialize)]
struct Guarded {
    #[serde(default)]
    replacement: Option<Vec<String>>,
    #[serde(default)]
    request_data: Option<Object>,
    #[serde(default)]
    user_api_key_dict: Option<Object>,
}

#[derive(Debug, Deserialize)]
struct RequestCall {
    data: Object,
    #[serde(flatten)]
    guarded: Guarded,
}

#[derive(Debug, Deserialize)]
struct ResponseCall {
    response: Object,
    #[serde(flatten)]
    guarded: Guarded,
}

#[derive(Debug, Deserialize)]
struct StreamCall {
    frames: Vec<Json>,
    #[serde(flatten)]
    guarded: Guarded,
}

#[derive(Debug, Deserialize)]
struct SseCall {
    chunks: Vec<SseChunk>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SseChunk {
    Text { value: String },
    Bytes { value: Vec<u8> },
}

impl SseChunk {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Text { value } => value.as_bytes(),
            Self::Bytes { value } => value,
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let calls: Vec<Value> = input.calls.iter().map(resolve).collect();
        Ok(json!({ "calls": calls }))
    }
}

fn resolve(call: &Call) -> Value {
    match call {
        Call::Request(given) => resolve_request(given),
        Call::Response(given) => resolve_response(given),
        Call::Stream(given) => resolve_stream(given),
        Call::Sse(given) => resolve_sse(given),
    }
}

fn resolve_request(given: &RequestCall) -> Value {
    let mut data = given.data.clone();
    let (locations, texts) = request_texts(&data);
    let called = !texts.is_empty();
    let request_data = called.then(|| data.clone());
    if let Some(replacement) = &given.guarded.replacement {
        write_texts(&mut data, &locations, replacement);
    }
    outcome(
        called.then_some(texts.as_slice()),
        request_data.as_ref(),
        &data,
    )
}

fn resolve_response(given: &ResponseCall) -> Value {
    let (locations, texts) = response_texts(&given.response);
    let called = !texts.is_empty();
    let merged = merge_context(
        &given.guarded,
        called,
        "response",
        Json::Object(given.response.clone()),
    );
    let mut response = given.response.clone();
    if let Some(replacement) = &given.guarded.replacement {
        write_texts(&mut response, &locations, replacement);
    }
    outcome(
        called.then_some(texts.as_slice()),
        merged.as_ref(),
        &response,
    )
}

fn resolve_stream(given: &StreamCall) -> Value {
    let accumulated = accumulated_stream_text(&given.frames);
    let called = !accumulated.is_empty();
    let texts = called.then(|| vec![accumulated]);
    let merged = merge_context(
        &given.guarded,
        called,
        "responses_so_far",
        Json::List(given.frames.clone()),
    );
    json!({
        "texts": texts.as_deref(),
        "request_data": merged.as_ref(),
        "data": Json::List(given.frames.clone()),
    })
}

fn outcome(texts: Option<&[String]>, request_data: Option<&Object>, data: &Object) -> Value {
    json!({
        "texts": texts,
        "request_data": request_data,
        "data": data,
    })
}

fn resolve_sse(given: &SseCall) -> Value {
    let chunks: Vec<&[u8]> = given.chunks.iter().map(SseChunk::bytes).collect();
    let texts = decode_google_guardrail_sse(&chunks);
    if texts.is_empty() {
        return json!({ "text": null });
    }
    json!({ "text": texts.concat() })
}

fn merge_context(
    guarded: &Guarded,
    called: bool,
    context_key: &str,
    context_value: Json,
) -> Option<Object> {
    let metadata = guarded.user_api_key_dict.clone().unwrap_or_default();
    called.then(|| {
        merged_request_data(
            guarded.request_data.as_ref(),
            &metadata,
            context_key,
            context_value,
        )
    })
}
