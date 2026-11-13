mod tests;

use std::fmt::Display;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_vertex::google_genai_guardrail::callbacks::merged_request_data;
use betterlite_vertex::google_genai_guardrail::records::{
    request_texts, response_texts, write_texts,
};
use betterlite_vertex::google_genai_guardrail::sse::{
    accumulated_stream_text, decode_google_guardrail_sse,
};
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

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
        let calls = input
            .calls
            .iter()
            .map(resolve)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "calls": calls }))
    }
}

fn failure<Error: Display>(error: Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn resolve(call: &Call) -> HelperResult {
    match call {
        Call::Request(given) => {
            let mut data = given.data.clone();
            let (locations, texts) = request_texts(&data);
            let called = !texts.is_empty();
            let request_data = called.then(|| data.clone());
            if let Some(replacement) = &given.guarded.replacement {
                write_texts(&mut data, &locations, replacement);
            }
            Ok(json!({
                "texts": called.then_some(texts.as_slice()),
                "request_data": request_data.as_ref(),
                "data": Json::Object(data),
            }))
        }
        Call::Response(given) => {
            let mut response = given.response.clone();
            let (locations, texts) = response_texts(&response);
            let called = !texts.is_empty();
            let metadata = given.guarded.user_api_key_dict.clone().unwrap_or_default();
            let merged = called.then(|| {
                merged_request_data(
                    given.guarded.request_data.as_ref(),
                    &metadata,
                    "response",
                    Json::Object(response.clone()),
                )
            });
            if let Some(replacement) = &given.guarded.replacement {
                write_texts(&mut response, &locations, replacement);
            }
            Ok(json!({
                "texts": called.then_some(texts.as_slice()),
                "request_data": merged.as_ref(),
                "data": Json::Object(response),
            }))
        }
        Call::Stream(given) => {
            let accumulated = accumulated_stream_text(&given.frames);
            let called = !accumulated.is_empty();
            let metadata = given.guarded.user_api_key_dict.clone().unwrap_or_default();
            let texts = called.then(|| vec![accumulated]);
            let merged = called.then(|| {
                merged_request_data(
                    given.guarded.request_data.as_ref(),
                    &metadata,
                    "responses_so_far",
                    Json::List(given.frames.clone()),
                )
            });
            Ok(json!({
                "texts": texts.as_deref(),
                "request_data": merged.as_ref(),
                "data": Json::List(given.frames.clone()),
            }))
        }
        Call::Sse(given) => {
            let chunks: Vec<&[u8]> = given.chunks.iter().map(SseChunk::bytes).collect();
            let texts = decode_google_guardrail_sse(&chunks);
            if texts.is_empty() {
                return Ok(json!({ "text": null }));
            }
            Ok(json!({ "text": texts.concat() }))
        }
    }
}
