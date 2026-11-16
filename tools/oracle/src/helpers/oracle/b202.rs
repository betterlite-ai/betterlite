use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo};
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults, Provider};
use betterlite_openai::text_completion_execution::catalog::{
    CompletionCatalog, CompletionRoute, route_for,
};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    chat_models: BTreeSet<String>,
    text_models: BTreeSet<String>,
    models: Vec<String>,
}

pub(crate) struct Op;

fn listed(model: &str) -> ModelDeployment {
    ModelDeployment {
        model_name: String::from("g"),
        litellm_params: LiteLlmParams {
            ssl_verify: None,
            model: Some(model.to_owned()),
            api_base: Some(String::from("http://127.0.0.1:9/v1")),
            api_key: Some(SecretString::from("oracle-key")),
            api_version: None,
            timeout: None,
            stream_timeout: None,
            max_retries: None,
            rpm: None,
            tpm: None,
            weight: None,
        },
        model_info: ModelInfo::default(),
    }
}

// only an openai prefix resolves with no catalog loaded; every other model takes the refusal shape the execution tests build
fn deployment(model: &str) -> Result<ChatDeployment, Box<HelperError>> {
    if model.starts_with("openai/") {
        return ChatDeployment::resolve(&listed(model), &OpenAiDefaults::default())
            .or_config_error();
    }
    let mut deployment =
        ChatDeployment::resolve(&listed("openai/gpt-4o"), &OpenAiDefaults::default())
            .or_config_error()?;
    model.clone_into(&mut deployment.litellm_model);
    deployment.provider = Provider::Unsupported {
        model: model.to_owned(),
    };
    Ok(deployment)
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let catalog = CompletionCatalog {
            chat_models: input.chat_models.clone(),
            text_models: input.text_models.clone(),
        };
        let mut selected = BTreeMap::new();
        for model in &input.models {
            let resolved = deployment(model)?;
            let route = route_for(&resolved, &catalog);
            // main.py prefixes only an openai model the chat catalog lacks; a text-catalog name keeps the name it arrived with
            let target = match &resolved.provider {
                Provider::OpenAi { .. } => match route {
                    CompletionRoute::Text { model: bare } => {
                        format!("text-completion-openai/{bare}")
                    }
                    CompletionRoute::Chat { .. } | CompletionRoute::Unsupported { .. } => {
                        model.clone()
                    }
                },
                Provider::Unsupported { .. } => model.clone(),
            };
            selected.insert(model.clone(), target);
        }
        serde_json::to_value(selected).map_err(|error| {
            Box::new(HelperError {
                class: String::from("InvalidOutput"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
