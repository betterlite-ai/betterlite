use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::copilot_chat::Settings;
use betterlite_core::api::copilot_chat::GithubCopilotConfig;
use betterlite_core::api::copilot_chat::messages::{CopilotMessage, transform_messages};
use betterlite_core::api::copilot_chat::native_reply::{NativeReply, transform_parsed_response};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use fake::{ListedReasoningModels, StoredKey};
use secrecy::SecretString;
use serde::Deserialize;
use tokio::runtime::{Builder, Runtime};
use uuid::Uuid;

use crate::helpers::{HelperError, HelperResult};

mod fake;
mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(crate) enum Input {
    TransformMessages {
        disable_system_to_assistant: bool,
        messages: Vec<CopilotMessage>,
    },
    ValidateEnvironment {
        api_key: Option<String>,
        copilot_key: String,
        headers: BTreeMap<String, String>,
        messages: Vec<CopilotMessage>,
    },
    SupportedParams {
        model: String,
        reasoning_models: Vec<String>,
    },
    RepairReply {
        response: NativeReply,
    },
}

pub(crate) struct Op;

fn failure(error: &dyn std::error::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: "CopilotChat".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| failure(&error))
}

fn config(
    api_key: Option<&str>,
    headers: &BTreeMap<String, String>,
    copilot_key: &str,
    reasoning_models: &[String],
) -> GithubCopilotConfig<StoredKey, ListedReasoningModels> {
    GithubCopilotConfig::new(
        api_key.map(|key| SecretString::from(key.to_owned())),
        headers.clone(),
        StoredKey::new(copilot_key),
        ListedReasoningModels::new(reasoning_models),
        Settings {
            disable_system_to_assistant: false,
            api_base: None,
        },
        CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
    )
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::TransformMessages {
                disable_system_to_assistant,
                messages,
            } => {
                let settings = Settings {
                    disable_system_to_assistant: *disable_system_to_assistant,
                    api_base: None,
                };
                serde_json::to_value(transform_messages(messages.clone(), &settings))
                    .map_err(|error| failure(&error))
            }
            Input::ValidateEnvironment {
                api_key,
                copilot_key,
                headers,
                messages,
            } => {
                let mut copilot = config(api_key.as_deref(), headers, copilot_key, &[]);
                let validated = runtime()?
                    .block_on(copilot.validate_environment(messages, Uuid::from_u128(1)))
                    .map_err(|error| failure(&error))?;
                serde_json::to_value(validated).map_err(|error| failure(&error))
            }
            Input::SupportedParams {
                model,
                reasoning_models,
            } => {
                let mut copilot = config(None, &BTreeMap::new(), "unused", reasoning_models);
                let params = runtime()?.block_on(copilot.supported_params(model));
                serde_json::to_value(params).map_err(|error| failure(&error))
            }
            Input::RepairReply { response } => {
                let repaired =
                    transform_parsed_response(response.clone()).map_err(|error| failure(&error))?;
                serde_json::to_value(repaired).map_err(|error| failure(&error))
            }
        }
    }
}
