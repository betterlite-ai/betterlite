use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::Json;
use betterlite_openai::moderation_request::ModerationInput;
use betterlite_openai::text_completion_policy::SUPPORTED_PARAMS;
use betterlite_openai::text_completion_request::{CompletionPrompt, CompletionRequest};
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::session::Entry;

mod tests;

// main.py reads these off the call and never hands them to completion as provider parameters
const CONTROL_PARAMETERS: [&str; 12] = [
    "allowed_fails",
    "caching",
    "content_policy_fallbacks",
    "context_window_fallbacks",
    "deployment_id",
    "fallbacks",
    "litellm_call_id",
    "max_fallbacks",
    "metadata",
    "model_info",
    "mock_response",
    "mock_tool_calls",
];
// omni-moderation-latest is the openai client's own default when create() is given no model
pub(crate) const DEFAULT_MODERATION_MODEL: &str = "omni-moderation-latest";
pub(crate) const OPENAI_PROVIDER: &str = "openai";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct TextCallWire {
    pub(crate) arguments: TextArguments,
    pub(crate) settings: SettingsWire,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct TextArguments {
    pub(crate) model: Option<String>,
    pub(crate) prompt: Option<Json>,
    pub(crate) stream: Option<bool>,
    pub(crate) timeout: Option<f64>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_version: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(flatten)]
    pub(crate) extra: IndexMap<String, Json>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SettingsWire {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) openai_key: Option<SecretString>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ModerationCallWire {
    pub(crate) arguments: ModerationArguments,
    #[serde(default)]
    pub(crate) settings: SettingsWire,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ModerationArguments {
    pub(crate) input: ModerationInput,
    pub(crate) model: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SessionWire {
    // absent means the process environment, the snapshot LiteLLM reads its globals from at import
    pub(crate) environment: Option<BTreeMap<String, String>>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the completion request could not be built: {source}")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error("betterlite does not support the {name} text completion parameter")]
    Unsupported { name: String },
    #[error(
        "Unmapped prompt format. Your prompt is neither a list of strings nor a string. File an issue - https://github.com/BerriAI/litellm/issues"
    )]
    Prompt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NativeTextErrorPayload {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) llm_provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) headers: BTreeMap<String, String>,
}

impl NativeTextErrorPayload {
    #[must_use]
    pub(crate) fn from_class(class: &ErrorClass) -> Self {
        let mut headers = BTreeMap::new();
        if let ErrorClass::RateLimit(details) = class {
            headers.clone_from(&details.headers);
        }
        let metadata = class.metadata();
        Self {
            kind: class_name(class).to_owned(),
            message: class_message(class),
            status_code: class_status(class),
            llm_provider: metadata.provider,
            model: metadata.model,
            headers,
        }
    }
}

pub(crate) fn body(arguments: &TextArguments) -> Result<IndexMap<String, Json>, ConvertError> {
    let mut body = IndexMap::new();
    let prompt_value = arguments.prompt.clone().unwrap_or(Json::Null);
    body.insert(String::from("prompt"), prompt_value);
    if let Some(stream) = arguments.stream {
        body.insert(String::from("stream"), Json::Bool(stream));
    }
    for (name, value) in &arguments.extra {
        let field = name.as_str();
        if CONTROL_PARAMETERS.contains(&field) {
            continue;
        }
        // completion lets num_retries stand in for max_retries, and it wins when both are given
        if field == "num_retries" {
            body.insert(String::from("max_retries"), value.clone());
            continue;
        }
        if field == "max_retries" && arguments.extra.contains_key("num_retries") {
            continue;
        }
        // the policy's provider parameters, then the three settings fields the transport takes itself
        if SUPPORTED_PARAMS.contains(&field) || ["best_of", "echo", "suffix"].contains(&field) {
            body.insert(field.to_owned(), value.clone());
            continue;
        }
        return Err(ConvertError::Unsupported {
            name: field.to_owned(),
        });
    }
    Ok(body)
}

pub(crate) fn completion_request(
    arguments: &TextArguments,
) -> Result<CompletionRequest, ConvertError> {
    let plain =
        serde_json::to_value(body(arguments)?).map_err(|source| ConvertError::Encode { source })?;
    serde_json::from_value(plain).map_err(|source| ConvertError::Body { source })
}

pub(crate) fn chat_messages(
    arguments: &TextArguments,
    mock: bool,
    provider: &str,
) -> Result<Vec<Json>, ConvertError> {
    let prompt =
        CompletionPrompt::from(serde_json::to_value(&arguments.prompt).unwrap_or_default());
    let takes_tokens = ["openai", "azure", "azure_text"].contains(&provider)
        || ["text-completion-openai", "text-completion-codestral"].contains(&provider);
    // main.py's token-prompt branch names these five providers, and a mock answer does not widen it
    let tokens = matches!(
        &prompt,
        CompletionPrompt::Tokens(_) | CompletionPrompt::TokenLists(_)
    );
    if tokens && !(mock && takes_tokens) {
        return Err(ConvertError::Prompt);
    }
    let messages = prompt.messages().ok_or(ConvertError::Prompt)?;
    messages
        .into_iter()
        .map(|mut message| {
            // the chat entry reads a content only as text or parts, and a mock answers without it
            if let Some(content) = message.get_mut("content")
                && content.is_array()
            {
                *content = serde_json::json!(content.to_string());
            }
            serde_json::from_value(message).map_err(|source| ConvertError::Body { source })
        })
        .collect()
}

// amoderation runs get_llm_provider before the call and sends its model, while moderation sends the value as given
#[must_use]
pub(crate) fn moderation_model(arguments: &ModerationArguments, entry: Entry) -> String {
    arguments.model.as_ref().map_or_else(
        || String::from(DEFAULT_MODERATION_MODEL),
        |model| match entry {
            Entry::Async => model.strip_prefix("openai/").unwrap_or(model).to_owned(),
            Entry::Sync => model.clone(),
        },
    )
}
