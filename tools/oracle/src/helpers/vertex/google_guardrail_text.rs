mod tests;

use std::fmt::Display;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_vertex::google_genai_guardrail::callbacks::{
    ApplyFuture, GenaiGuardrail, GuardrailCall, GuardrailInputs, InputType, process_request,
    process_response, process_stream,
};
use betterlite_vertex::google_genai_guardrail::sse::decode_google_guardrail_sse;
use betterlite_vertex::google_native_content::response::GenerateContentResponse;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
struct RequestCall {
    data: Object,
    #[serde(default)]
    replacement: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseCall {
    response: Object,
    #[serde(default)]
    replacement: Option<Vec<String>>,
    #[serde(default)]
    request_data: Option<Object>,
    #[serde(default)]
    user_api_key_dict: Option<Object>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StreamCall {
    frames: Vec<Json>,
    #[serde(default)]
    replacement: Option<Vec<String>>,
    #[serde(default)]
    request_data: Option<Object>,
    #[serde(default)]
    user_api_key_dict: Option<Object>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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

#[derive(Debug, thiserror::Error)]
#[error("the recording guardrail cannot fail")]
struct NeverFails;

#[derive(Debug, Clone)]
struct RecordedCall {
    texts: Vec<String>,
    model: Option<String>,
    input_type: InputType,
    request_data: Object,
}

struct Recorder {
    replacement: Option<Vec<String>>,
    calls: Vec<RecordedCall>,
}

impl GenaiGuardrail for Recorder {
    type Failure = NeverFails;

    fn apply_guardrail<'call>(
        &'call mut self,
        inputs: GuardrailInputs,
        call: GuardrailCall<'call>,
    ) -> ApplyFuture<'call, Self::Failure> {
        self.calls.push(RecordedCall {
            texts: inputs.texts.clone(),
            model: inputs.model.clone(),
            input_type: call.input_type,
            request_data: call.request_data.clone(),
        });
        let replacement = match self.replacement.clone() {
            Some(replacement) => replacement,
            None => inputs.texts,
        };
        Box::pin(async move { Ok(Some(replacement)) })
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(failure)?;
        let calls = input
            .calls
            .iter()
            .map(|call| resolve(&runtime, call))
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

fn resolve(runtime: &tokio::runtime::Runtime, call: &Call) -> HelperResult {
    match call {
        Call::Request(given) => request_case(runtime, given),
        Call::Response(given) => response_case(runtime, given),
        Call::Stream(given) => stream_case(runtime, given),
        Call::Sse(given) => sse_case(given),
    }
}

fn request_case(runtime: &tokio::runtime::Runtime, call: &RequestCall) -> HelperResult {
    let mut data = call.data.clone();
    let mut recorder = Recorder {
        replacement: call.replacement.clone(),
        calls: Vec::new(),
    };
    runtime
        .block_on(process_request(&mut data, &mut recorder))
        .map_err(failure)?;
    result(&recorder, Json::Object(data))
}

fn response_case(runtime: &tokio::runtime::Runtime, call: &ResponseCall) -> HelperResult {
    let mut record = GenerateContentResponse {
        wire: call.response.clone(),
    };
    let user_metadata = call.user_api_key_dict.clone().unwrap_or_default();
    let mut recorder = Recorder {
        replacement: call.replacement.clone(),
        calls: Vec::new(),
    };
    runtime
        .block_on(process_response(
            &mut record.wire,
            call.request_data.as_ref(),
            &user_metadata,
            &mut recorder,
        ))
        .map_err(failure)?;
    result(&recorder, Json::Object(record.wire))
}

fn stream_case(runtime: &tokio::runtime::Runtime, call: &StreamCall) -> HelperResult {
    let frames = call.frames.clone();
    let user_metadata = call.user_api_key_dict.clone().unwrap_or_default();
    let mut recorder = Recorder {
        replacement: call.replacement.clone(),
        calls: Vec::new(),
    };
    runtime
        .block_on(process_stream(
            &frames,
            call.request_data.as_ref(),
            &user_metadata,
            &mut recorder,
        ))
        .map_err(failure)?;
    result(&recorder, Json::List(frames))
}

fn sse_case(call: &SseCall) -> HelperResult {
    let chunks: Vec<&[u8]> = call.chunks.iter().map(SseChunk::bytes).collect();
    let texts = decode_google_guardrail_sse(&chunks);
    if texts.is_empty() {
        return Ok(json!({ "text": null }));
    }
    Ok(json!({ "text": texts.concat() }))
}

fn result(recorder: &Recorder, data: Json) -> HelperResult {
    if recorder.calls.is_empty() {
        return Ok(json!({
            "called": false,
            "call_count": 0,
            "texts": null,
            "model": null,
            "input_type": null,
            "request_data": null,
            "data": &data,
        }));
    }
    let call = &recorder.calls[0];
    Ok(json!({
        "called": true,
        "call_count": recorder.calls.len(),
        "texts": &call.texts,
        "model": &call.model,
        "input_type": call.input_type.as_str(),
        "request_data": &call.request_data,
        "data": &data,
    }))
}
