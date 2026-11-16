use std::fmt::Display;
use std::sync::PoisonError;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_openai::huggingface_embedding::error::HuggingfaceEmbeddingError;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::model_hub_dispatch::HubDispatchError;
use betterlite_redact::secret::Redactor;
use serde_json::{Map, Value, json};

use super::fixture::{Recorder, Wire};

// LiteLLM's helper records these headers; the HTTP client adds the rest on its own
const KEPT_HEADERS: [&str; 2] = ["authorization", "content-type"];

pub(super) fn helper_error(class: &str, error: &dyn Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn wire_value(wire: &Wire) -> Value {
    let mut headers = Map::new();
    for (name, value) in &wire.headers {
        let lowered = name.to_lowercase();
        if KEPT_HEADERS.contains(&lowered.as_str()) || lowered.starts_with("x-") {
            headers.insert(lowered, Value::String(value.clone()));
        }
    }
    let body = wire
        .body
        .as_deref()
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
        .unwrap_or(Value::Null);
    json!({"method": wire.method, "url": wire.url, "headers": headers, "body": body})
}

pub(super) fn answer(recorder: &Recorder, error: &Value) -> Value {
    let requests: Vec<Value> = recorder
        .sent
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(wire_value)
        .collect();
    json!({"requests": requests, "error": error})
}

// the status, class name and text LiteLLM's provider exception carries for this error
fn classified(error: &HubDispatchError) -> (Option<u16>, &'static str, String) {
    match error {
        HubDispatchError::Embedding(HuggingfaceEmbeddingError::Huggingface(failure)) => (
            Some(failure.status_code),
            "HuggingFaceError",
            failure.message.clone(),
        ),
        HubDispatchError::Replicate(failure) => failure.api().map_or_else(
            || (None, "ReplicateError", failure.to_string()),
            |api| (Some(api.status_code), "ReplicateError", api.message.clone()),
        ),
        HubDispatchError::Predibase(failure) => (
            Some(failure.status_code),
            "PredibaseError",
            failure.message.clone(),
        ),
        HubDispatchError::PredibaseSetup(_) | HubDispatchError::FieldType { .. } => {
            (None, "ValueError", error.to_string())
        }
        HubDispatchError::Chat(_)
        | HubDispatchError::Embedding(_)
        | HubDispatchError::Rerank(_)
        | HubDispatchError::Operation(_)
        | HubDispatchError::Url { .. }
        | HubDispatchError::Headers { .. }
        | HubDispatchError::Encode { .. }
        | HubDispatchError::BodyNotObject
        | HubDispatchError::BodyNotUtf8 { .. }
        | HubDispatchError::PreCall { .. }
        | HubDispatchError::Environment { .. }
        | HubDispatchError::MissingVocabulary
        | HubDispatchError::Encoding { .. } => (None, "Exception", error.to_string()),
    }
}

// the status LiteLLM's exception_type gives the provider error this module raised
pub(super) fn error_status(error: &HubDispatchError, provider: &str, redactor: &Redactor) -> Value {
    let (status_code, exception_type, message) = classified(error);
    let raw = RawException {
        model: String::new(),
        custom_llm_provider: provider.to_owned(),
        error_str: message.clone(),
        status_code,
        status_code_is_synthesized: false,
        message: Some(message),
        body: None,
        exception_type: exception_type.to_owned(),
    };
    let class = dispatch(&DispatchInput::Raw(Box::new(raw)), redactor);
    json!({"status": class.metadata().status_code})
}
