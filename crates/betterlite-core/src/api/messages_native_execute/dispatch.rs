use std::sync::Arc;

use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::entry::MessagesEntryResult;
use super::error::{MessagesExecuteError, RaisedException};

// functools.partial's keyword arguments: a hook extra with one of these names is a duplicate keyword
const NAMED_KEYWORDS: [&str; 18] = [
    "max_tokens",
    "messages",
    "model",
    "metadata",
    "stop_sequences",
    "stream",
    "system",
    "temperature",
    "thinking",
    "tool_choice",
    "tools",
    "top_k",
    "top_p",
    "api_key",
    "api_base",
    "client",
    "custom_llm_provider",
    "_litellm_messages_presanitized",
];

// anthropic_messages_handler's arguments, with is_async and the presanitized marker already applied
pub struct MessagesDispatchRequest {
    pub max_tokens: i64,
    pub messages: Vec<Json>,
    pub model: String,
    pub metadata: Option<Json>,
    pub stop_sequences: Option<Json>,
    pub stream: Option<bool>,
    pub system: Option<Json>,
    pub temperature: Option<Json>,
    pub thinking: Option<Json>,
    pub tool_choice: Option<Json>,
    pub tools: Option<Vec<Json>>,
    pub top_k: Option<Json>,
    pub top_p: Option<Json>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub client: Option<ProxiedClient>,
    pub custom_llm_provider: Option<String>,
    pub kwargs: IndexMap<String, Json>,
    pub presanitized: bool,
}

// LiteLLM's BaseLLMException, as the final execution raises it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagesBaseLlmException {
    pub status_code: u16,
    pub message: String,
}

// What the composing caller hands back after it executed the prepared dispatch
pub enum MessagesDispatchOutcome {
    Returned(MessagesEntryResult),
    BaseLlmException(MessagesBaseLlmException),
    Raised(RaisedException),
}

// The prepared final dispatch: the caller executes it and resumes this pipeline's result handling
pub struct MessagesExecutionContinuation {
    pub dispatch: MessagesDispatchRequest,
    pub redactor: Arc<Redactor>,
}

impl MessagesExecutionContinuation {
    pub fn prepare(
        dispatch: MessagesDispatchRequest,
        redactor: Arc<Redactor>,
    ) -> Result<Self, MessagesExecuteError> {
        let duplicate = dispatch
            .kwargs
            .keys()
            .cloned()
            .find(|key| NAMED_KEYWORDS.contains(&key.as_str()));
        if let Some(name) = duplicate {
            return Err(MessagesExecuteError::DuplicateKeyword { name });
        }
        Ok(Self { dispatch, redactor })
    }

    // Only a BaseLLMException is converted here; an already raised error passes through untouched
    pub fn resume(
        &self,
        outcome: MessagesDispatchOutcome,
    ) -> Result<MessagesEntryResult, MessagesExecuteError> {
        match outcome {
            MessagesDispatchOutcome::Returned(result) => Ok(result),
            MessagesDispatchOutcome::Raised(raised) => Err(MessagesExecuteError::Raised(raised)),
            MessagesDispatchOutcome::BaseLlmException(exception) => {
                let raw = RawException {
                    model: self.dispatch.model.clone(),
                    custom_llm_provider: self
                        .dispatch
                        .custom_llm_provider
                        .clone()
                        .unwrap_or_default(),
                    error_str: exception.message.clone(),
                    status_code: Some(exception.status_code),
                    status_code_is_synthesized: false,
                    message: Some(exception.message),
                    body: None,
                    exception_type: "BaseLLMException".to_owned(),
                };
                let class = dispatch(&DispatchInput::Raw(Box::new(raw)), self.redactor.as_ref());
                Err(MessagesExecuteError::Classified(Box::new(class)))
            }
        }
    }
}
