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

#[derive(Debug, thiserror::Error)]
#[error("the recording guardrail cannot fail")]
struct NeverFails;

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
        let replacement = self.replacement.clone().unwrap_or(inputs.texts);
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
        Call::Request(given) => {
            let mut recorder = Recorder {
                replacement: given.guarded.replacement.clone(),
                calls: Vec::new(),
            };
            let mut data = given.data.clone();
            runtime
                .block_on(process_request(&mut data, &mut recorder))
                .map_err(failure)?;
            result(&recorder, &Json::Object(data))
        }
        Call::Response(given) => {
            let mut recorder = Recorder {
                replacement: given.guarded.replacement.clone(),
                calls: Vec::new(),
            };
            let metadata = given.guarded.user_api_key_dict.clone().unwrap_or_default();
            let mut record = GenerateContentResponse {
                wire: given.response.clone(),
            };
            runtime
                .block_on(process_response(
                    &mut record.wire,
                    given.guarded.request_data.as_ref(),
                    &metadata,
                    &mut recorder,
                ))
                .map_err(failure)?;
            result(&recorder, &Json::Object(record.wire))
        }
        Call::Stream(given) => {
            let frames = given.frames.clone();
            let metadata = given.guarded.user_api_key_dict.clone().unwrap_or_default();
            let mut recorder = Recorder {
                replacement: given.guarded.replacement.clone(),
                calls: Vec::new(),
            };
            runtime
                .block_on(process_stream(
                    &frames,
                    given.guarded.request_data.as_ref(),
                    &metadata,
                    &mut recorder,
                ))
                .map_err(failure)?;
            result(&recorder, &Json::List(frames))
        }
        Call::Sse(given) => sse_case(given),
    }
}

fn sse_case(call: &SseCall) -> HelperResult {
    let chunks: Vec<&[u8]> = call.chunks.iter().map(SseChunk::bytes).collect();
    let texts = decode_google_guardrail_sse(&chunks);
    if texts.is_empty() {
        return Ok(json!({ "text": null }));
    }
    Ok(json!({ "text": texts.concat() }))
}

fn result(recorder: &Recorder, data: &Json) -> HelperResult {
    let Some(call) = recorder.calls.first() else {
        return Ok(json!({
            "called": false,
            "call_count": 0,
            "texts": null,
            "model": null,
            "input_type": null,
            "request_data": null,
            "data": data,
        }));
    };
    Ok(json!({
        "called": true,
        "call_count": recorder.calls.len(),
        "texts": &call.texts,
        "model": &call.model,
        "input_type": call.input_type.as_str(),
        "request_data": &call.request_data,
        "data": data,
    }))
}
