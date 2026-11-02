use crate::error::details::{ResponseDetails, TimeoutDetails, TimeoutStatus};
use crate::error::{ErrorClass, ErrorContext};

mod tests;

// a dict original_exception contributes its "error" key (or "" when absent), anything else its str() form; TextStatus is a text cause that also carries the response LiteLLM's OllamaError attaches, kept as its own variant so the existing Text shape native.rs's dispatch table builds stays untouched
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OllamaOriginalException {
    Dict {
        error: String,
    },
    Text {
        text: String,
    },
    TextStatus {
        text: String,
        #[serde(default)]
        status_code: Option<u16>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OllamaExceptionInput {
    pub model: String,
    pub original_exception: OllamaOriginalException,
}

// mirrors _map_ollama_exception's independent `if`/`elif` branches, each passing response=getattr(original_exception, "response", None); native.rs's dispatch table always builds a Text cause, which keeps the class default status since that call site sits outside this task's scope
#[must_use]
pub fn map_ollama_exception(input: &OllamaExceptionInput) -> Option<ErrorClass> {
    let (error_str, status_code) = match &input.original_exception {
        OllamaOriginalException::Dict { error } => (error.as_str(), None),
        OllamaOriginalException::Text { text } => (text.as_str(), None),
        OllamaOriginalException::TextStatus { text, status_code } => (text.as_str(), *status_code),
    };
    let context = || ErrorContext {
        message: format!("ollama: {error_str}"),
        model: Some(input.model.clone()),
        provider: Some("ollama".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    if error_str.contains("no such file or directory") {
        return Some(ErrorClass::BadRequest(Box::new(
            ResponseDetails::with_status(context(), status_code),
        )));
    }
    if error_str.contains("Failed to establish a new connection") {
        return Some(ErrorClass::ServiceUnavailable(Box::new(context())));
    }
    if error_str.contains("Invalid response object from API") {
        return Some(ErrorClass::BadRequest(Box::new(
            ResponseDetails::with_status(context(), status_code),
        )));
    }
    if error_str.contains("Read timed out") {
        return Some(ErrorClass::Timeout(Box::new(TimeoutDetails {
            context: context(),
            original_status: TimeoutStatus(408),
        })));
    }
    None
}
