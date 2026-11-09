use std::sync::Mutex;

use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::builtin::{Field, Message};
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::Hub;
use betterlite_prompts::factory::hub::builtin_entries;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::helpers::HelperError;

#[derive(Deserialize)]
pub(crate) struct UrlCase {
    #[serde(default)]
    pub(super) api_base: Option<String>,
    pub(super) stream: bool,
}

#[derive(Deserialize)]
pub(crate) struct MapCase {
    pub(super) non_default: IndexMap<String, Field>,
    #[serde(default)]
    pub(super) optional: IndexMap<String, Field>,
}

#[derive(Deserialize)]
pub(crate) struct RequestCase {
    pub(super) model: String,
    pub(super) messages: Vec<Message>,
    #[serde(default)]
    pub(super) options: IndexMap<String, Field>,
}

#[derive(Deserialize)]
pub(crate) struct ReplyCase {
    pub(super) status: u16,
    pub(super) body: String,
}

#[derive(Deserialize)]
pub(crate) struct EmbeddingReplyCase {
    pub(super) status: u16,
    pub(super) body: String,
    pub(super) model: String,
    pub(super) input: EmbeddingInput,
}

#[derive(Deserialize)]
pub(crate) struct SplitCase {
    pub(super) data: Vec<f64>,
    pub(super) shape: Vec<i64>,
}

pub(super) fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
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
