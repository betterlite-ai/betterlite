use std::collections::BTreeMap;
use std::error::Error;

use betterlite_config::sections::copilot_chat::Settings;
use betterlite_core::api::copilot_embedding::EmbeddingConfig;
use betterlite_core::api::copilot_embedding::request::{
    map_openai_params, transform_embedding_request,
};
use betterlite_core::api::copilot_messages::{MessagesConfig, MessagesRequest};
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod fake;
mod tests;

use fake::StoredKey;

#[derive(Debug, Deserialize)]
pub(crate) struct EmbeddingCall {
    model: String,
    input: EmbeddingInput,
    non_default_params: Object,
    optional_params: Object,
    headers: BTreeMap<String, String>,
    // None means the cached record carries no token, which LiteLLM renders the same way
    copilot_key: Option<String>,
    api_base: Option<String>,
    cached_base: Option<String>,
    env_base: Option<String>,
}

// LiteLLM discards the caller's base, so the input carries none: the case still sends one and
// serde drops it, which proves the resolved session host is what receives the token.
#[derive(Debug, Deserialize)]
pub(crate) struct MessagesCall {
    model: String,
    headers: BTreeMap<String, String>,
    optional_params: IndexMap<String, Json>,
    messages: Vec<Json>,
    copilot_key: Option<String>,
    cached_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Input {
    Embedding(EmbeddingCall),
    Messages(MessagesCall),
}

pub(crate) struct Op;

fn failure(error: &dyn Error, status: Option<u16>) -> Box<HelperError> {
    Box::new(HelperError {
        class: "CopilotAuxiliary".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| failure(&error, None))
}

fn embedding(call: &EmbeddingCall) -> HelperResult {
    let settings = Settings {
        disable_system_to_assistant: false,
        api_base: call.env_base.clone(),
    };
    let mut config = EmbeddingConfig {
        credentials: StoredKey::new(call.copilot_key.as_deref(), call.cached_base.as_deref()),
        settings,
    };
    let runtime = runtime()?;
    let headers = runtime
        .block_on(config.validate_environment(call.headers.clone(), &call.model))
        .map_err(|error| failure(&error, error.status()))?;
    let url = runtime.block_on(config.get_complete_url(call.api_base.as_deref()));
    let mut optional = call.optional_params.clone();
    map_openai_params(&call.non_default_params, &mut optional);
    let body = transform_embedding_request(&call.model, &call.input, &optional);
    Ok(json!({"headers": headers, "url": url, "body": body}))
}

fn messages(call: &MessagesCall) -> HelperResult {
    let mut config = MessagesConfig {
        credentials: StoredKey::new(call.copilot_key.as_deref(), call.cached_base.as_deref()),
    };
    let request = MessagesRequest {
        optional_params: &call.optional_params,
        messages: &call.messages,
    };
    let runtime = runtime()?;
    let environment = runtime
        .block_on(config.validate_environment(call.headers.clone(), &request, &call.model))
        .map_err(|error| failure(&error, error.status()))?;
    let url = runtime.block_on(config.get_complete_url(Some(environment.api_base.as_str())));
    Ok(json!({
        "headers": environment.headers,
        "api_base": environment.api_base,
        "url": url,
    }))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Embedding(call) => embedding(call),
            Input::Messages(call) => messages(call),
        }
    }
}
