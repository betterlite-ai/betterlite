use betterlite_error::error::{ErrorClass, class_message};
use betterlite_models::decisions::Json;
use betterlite_openai::moderation_request::ModerationInput;
use betterlite_openai::text_completion_request::{CompletionPrompt, CompletionRequest};
use betterlite_sdk::call_wrapper::error::{class_name, class_status};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::session::Entry;

mod tests;

// main.py's own refusal of a prompt it cannot send; Python's rendering of the prompt value is not ported
pub(crate) const UNMAPPED_PROMPT: &str = "Unmapped prompt format. Your prompt is neither a list of strings nor a string. File an issue - https://github.com/BerriAI/litellm/issues";
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
// the completion body completion() builds from text_completion's named arguments and its kwargs
const TEXT_PARAMETERS: [&str; 22] = [
    "best_of",
    "echo",
    "extra_headers",
    "frequency_penalty",
    "function_call",
    "functions",
    "logit_bias",
    "logprobs",
    "max_retries",
    "max_tokens",
    "n",
    "presence_penalty",
    "response_format",
    "seed",
    "stop",
    "stream_options",
    "suffix",
    "temperature",
    "tools",
    "tool_choice",
    "top_p",
    "user",
];
// omni-moderation-latest is the openai client's own default when create() is given no model
pub(crate) const DEFAULT_MODERATION_MODEL: &str = "omni-moderation-latest";
pub(crate) const OPENAI_PROVIDER: &str = "openai";

// The one text a native method takes: the call's arguments and the settings snapshot main.py read.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct TextCallWire {
    pub(crate) arguments: TextArguments,
    pub(crate) settings: TextSettingsWire,
}

// main.py's named arguments, the merged headers and every other kwarg with the shape it arrived in.
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
    // every other kwarg, which completion hands to the provider once the parameter list accepts it
    #[serde(flatten)]
    pub(crate) extra: IndexMap<String, Json>,
}

// the module-level values main.py's text completion reads before it dispatches
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct TextSettingsWire {
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) openai_key: Option<SecretString>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ModerationCallWire {
    pub(crate) arguments: ModerationArguments,
    #[serde(default)]
    pub(crate) settings: ModerationSettingsWire,
}

// moderation's three named arguments and the api_base its kwargs carry
#[derive(Debug, Deserialize)]
pub(crate) struct ModerationArguments {
    pub(crate) input: ModerationInput,
    pub(crate) model: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ModerationSettingsWire {
    pub(crate) api_key: Option<SecretString>,
    pub(crate) openai_key: Option<SecretString>,
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

// The JSON text NativeTextError carries: LiteLLM's exception name and the fields its class keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NativeTextErrorPayload {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) llm_provider: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) body: Option<Json>,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) num_retries: Option<i64>,
    pub(crate) max_retries: Option<i64>,
}

impl NativeTextErrorPayload {
    #[must_use]
    pub(crate) fn from_class(class: &ErrorClass) -> Self {
        let metadata = class.metadata();
        let mut headers = BTreeMap::new();
        if let ErrorClass::RateLimit(details) = class {
            headers.clone_from(&details.headers);
        }
        Self {
            kind: class_name(class).to_owned(),
            message: class_message(class),
            // a plain Exception carries no status_code; every LiteLLM error class does
            status_code: class_status(class),
            llm_provider: metadata.provider,
            model: metadata.model,
            body: None,
            headers,
            num_retries: metadata.retry.num_retries.map(i64::from),
            max_retries: metadata.retry.max_retries.map(i64::from),
        }
    }
}

// main.py's prompt dispatch: the client's value read as the one completion prompt it names.
#[must_use]
pub(crate) fn prompt(arguments: &TextArguments) -> CompletionPrompt {
    let value = arguments.prompt.clone().unwrap_or(Json::Null);
    let plain = serde_json::to_value(value).unwrap_or_default();
    CompletionPrompt::from(plain)
}

// The completion body: the prompt, the stream flag and every provider parameter the call named.
pub(crate) fn body(arguments: &TextArguments) -> Result<IndexMap<String, Json>, ConvertError> {
    let mut body = IndexMap::new();
    body.insert(
        String::from("prompt"),
        arguments.prompt.clone().unwrap_or(Json::Null),
    );
    if let Some(stream) = arguments.stream {
        body.insert(String::from("stream"), Json::Bool(stream));
    }
    for (name, value) in &arguments.extra {
        let name = name.as_str();
        if CONTROL_PARAMETERS.contains(&name) {
            continue;
        }
        if TEXT_PARAMETERS.contains(&name) {
            body.insert(name.to_owned(), value.clone());
            continue;
        }
        // completion hands num_retries to the client as its max_retries
        if name == "num_retries" {
            if !arguments.extra.contains_key("max_retries") {
                body.insert(String::from("max_retries"), value.clone());
            }
            continue;
        }
        return Err(ConvertError::Unsupported {
            name: name.to_owned(),
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

// main.py turns a string prompt, or one user message per string, into the chat call's messages.
pub(crate) fn chat_messages(arguments: &TextArguments) -> Result<Vec<Json>, ConvertError> {
    let prompt = prompt(arguments);
    match &prompt {
        CompletionPrompt::Text(_) | CompletionPrompt::Texts(_) => {}
        CompletionPrompt::Tokens(_)
        | CompletionPrompt::TokenLists(_)
        | CompletionPrompt::Empty
        | CompletionPrompt::Unmapped(_)
        | CompletionPrompt::Unreadable(_) => return Err(ConvertError::Prompt),
    }
    let Some(messages) = prompt.messages() else {
        return Err(ConvertError::Prompt);
    };
    messages
        .into_iter()
        .map(|message| {
            serde_json::from_value(message).map_err(|source| ConvertError::Body { source })
        })
        .collect()
}

// amoderation runs get_llm_provider before the call and sends its model, while moderation sends the value as given
#[must_use]
pub(crate) fn moderation_model(arguments: &ModerationArguments, entry: Entry) -> String {
    match &arguments.model {
        Some(model) => match entry {
            Entry::Async => model.strip_prefix("openai/").unwrap_or(model).to_owned(),
            Entry::Sync => model.clone(),
        },
        None => String::from(DEFAULT_MODERATION_MODEL),
    }
}
