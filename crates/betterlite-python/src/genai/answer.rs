use super::convert::json_text;
use super::session::Outcome;
use super::stream::StreamKind;
use betterlite_core::api::genai::chat_call::{
    CompletionCall, CompletionExecutor, CompletionOutput, GenerateContentOutcome as AdapterOutcome,
};
use betterlite_core::api::genai::generate_content::GenerateContentAnswer;
use betterlite_core::api::genai_http::service::{
    AsyncGenerateContentOutcome, GenerateContentOutcome as NativeOutcome,
};
use betterlite_prompts::arguments::json::Json;
use betterlite_vertex::google_native_content::response::GenerateContentResponse;
use pyo3::PyErr;
use pyo3::exceptions::PyRuntimeError;
use std::future::{Future, ready};
const ADAPTER_UNAVAILABLE: &str =
    "litellm.<operation>: the chat completion adapter is not part of betterlite's Python package";
// The chat adapter every non-Gemini, non-Vertex model would run through; the Python package ships no executor for it.
pub(crate) struct AdapterUnavailable;
#[derive(Debug, thiserror::Error)]
#[error("the chat completion adapter is not part of betterlite's Python package")]
pub(crate) struct AdapterError;
impl CompletionExecutor for AdapterUnavailable {
    type Error = AdapterError;
    fn completion(&self, _call: CompletionCall) -> Result<CompletionOutput, AdapterError> {
        Err(AdapterError)
    }
    fn acompletion(
        &self,
        _call: CompletionCall,
    ) -> impl Future<Output = Result<CompletionOutput, AdapterError>> + Send {
        ready(Err(AdapterError))
    }
}
// the answer in the shape Python reads: JSON text, or bytes it iterates
pub(crate) fn answered(
    answer: GenerateContentAnswer<GenerateContentResponse>,
) -> Result<Outcome, PyErr> {
    match answer {
        GenerateContentAnswer::Mock(json) => Ok(Outcome::Text(json_text(&json)?)),
        GenerateContentAnswer::Adapter(outcome) => adapter_answer(outcome),
        GenerateContentAnswer::Native(outcome) => native_answer(outcome),
        GenerateContentAnswer::AsyncNative(outcome) => async_answer(outcome),
    }
}
fn adapter_answer(answer: AdapterOutcome) -> Result<Outcome, PyErr> {
    match answer {
        AdapterOutcome::Response(response) => {
            let encoded = serde_json::to_value(response)
                .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
            let json: Json = serde_json::from_value(encoded)
                .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
            Ok(Outcome::Text(json_text(&json)?))
        }
        // AdapterUnavailable refuses every call, so the adapter never answers a stream here
        AdapterOutcome::Stream(_stream) => Err(PyRuntimeError::new_err(ADAPTER_UNAVAILABLE)),
    }
}
fn native_answer(answer: NativeOutcome<GenerateContentResponse>) -> Result<Outcome, PyErr> {
    match answer {
        NativeOutcome::Response(response) => {
            Ok(Outcome::Text(json_text(&Json::Object(response.wire))?))
        }
        NativeOutcome::Stream(stream) => Ok(Outcome::Stream(StreamKind::Blocking(stream.iterator))),
    }
}
fn async_answer(
    answer: AsyncGenerateContentOutcome<GenerateContentResponse>,
) -> Result<Outcome, PyErr> {
    match answer {
        AsyncGenerateContentOutcome::Response(response) => {
            Ok(Outcome::Text(json_text(&Json::Object(response.wire))?))
        }
        AsyncGenerateContentOutcome::Stream(stream) => {
            Ok(Outcome::Stream(StreamKind::Asynchronous(stream.iterator)))
        }
    }
}
