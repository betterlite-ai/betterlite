use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::Message;
use betterlite_provider::ollama_chat::config::OllamaChatConfig;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::helpers::HelperError;

#[derive(Deserialize)]
pub(crate) struct ParamCase {
    pub(super) model: String,
    pub(super) non_default: IndexMap<String, Json>,
    #[serde(default)]
    pub(super) optional: IndexMap<String, Json>,
}

#[derive(Deserialize)]
pub(crate) struct ValidateCase {
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub(crate) struct RequestCase {
    pub(super) model: String,
    pub(super) messages: Vec<Message>,
    #[serde(default)]
    pub(super) options: IndexMap<String, Json>,
    #[serde(default)]
    pub(super) config: OllamaChatConfig,
}

#[derive(Deserialize)]
pub(crate) struct ReplyCase {
    pub(super) model: String,
    pub(super) body: String,
    #[serde(default)]
    pub(super) request_data: IndexMap<String, Json>,
    #[serde(default)]
    pub(super) litellm_params: IndexMap<String, Json>,
}

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| {
            Box::new(HelperError {
                class: String::from("Runtime"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
}
