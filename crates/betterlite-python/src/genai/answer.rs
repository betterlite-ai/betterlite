use super::session::{GenaiCore, Outcome};
use super::stream::{NativeGenaiStream, StreamKind};
use betterlite_core::api::genai::chat_call::{
    CompletionCall, CompletionExecutor, CompletionOutput, GenerateContentOutcome as AdapterOutcome,
};
use betterlite_core::api::genai::generate_content::GenerateContentAnswer;
use betterlite_core::api::genai_http::service::{
    AsyncGenerateContentOutcome, GenerateContentOutcome as NativeOutcome,
};
use betterlite_prompts::arguments::json::Json;
use betterlite_vertex::genai::session::Entry;
use betterlite_vertex::genai::session::call::GenaiCall;
use betterlite_vertex::google_native_content::response::GenerateContentResponse;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyString;
use pyo3_async_runtimes::tokio::future_into_py;
use std::future::{Future, ready};
use std::sync::Arc;
mod tests;
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
        GenerateContentAnswer::Mock(json) => text_outcome(&json),
        GenerateContentAnswer::Adapter(AdapterOutcome::Response(response)) => {
            let encoded = serde_json::to_value(response)
                .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
            text_outcome(
                &serde_json::from_value(encoded)
                    .map_err(|error| PyRuntimeError::new_err(error.to_string()))?,
            )
        }
        // AdapterUnavailable refuses every call, so the adapter never answers a stream here
        GenerateContentAnswer::Adapter(AdapterOutcome::Stream(_stream)) => {
            Err(PyRuntimeError::new_err(ADAPTER_UNAVAILABLE))
        }
        GenerateContentAnswer::Native(NativeOutcome::Response(response))
        | GenerateContentAnswer::AsyncNative(AsyncGenerateContentOutcome::Response(response)) => {
            text_outcome(&Json::Object(response.wire))
        }
        GenerateContentAnswer::Native(NativeOutcome::Stream(stream)) => Ok(Outcome::Stream(
            StreamKind::Blocking(Box::new(stream.iterator)),
        )),
        GenerateContentAnswer::AsyncNative(AsyncGenerateContentOutcome::Stream(stream)) => Ok(
            Outcome::Stream(StreamKind::Asynchronous(Box::new(stream.iterator))),
        ),
    }
}
// the coroutine both async entry points hand back; it runs the call on the asyncio loop's runtime
pub(crate) fn awaiting(
    py: Python<'_>,
    core: Arc<GenaiCore>,
    entry: Entry,
    call: GenaiCall,
) -> PyResult<Bound<'_, PyAny>> {
    future_into_py(py, async move {
        let outcome = Box::pin(core.call(call, entry)).await?;
        Python::attach(|attached| outcome_object(attached, outcome))
    })
}
// the answer Python reads: the response text it loads into a dict, or the stream object it iterates
pub(crate) fn outcome_object(py: Python<'_>, outcome: Outcome) -> PyResult<Py<PyAny>> {
    match outcome {
        Outcome::Text(text) => Ok(PyString::new(py, &text).into_any().unbind()),
        Outcome::Stream(kind) => Ok(Py::new(py, NativeGenaiStream::from(kind))?.into_any()),
    }
}
// the response body as the JSON text Python loads; LiteLLM hands back the model_dump of the same document
fn text_outcome(json: &Json) -> Result<Outcome, PyErr> {
    let text =
        serde_json::to_string(json).map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
    Ok(Outcome::Text(text))
}
