use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::identity::anthropic_text::anthropic_text_provider;
use betterlite_models::identity::azure::is_azure_claude_model;
use betterlite_models::identity::cohere::cohere_chat_provider;
use betterlite_models::identity::endpoint::endpoint_matches_api_base;
use betterlite_models::identity::explicit_prefix::{PrefixClassificationInput, classify_prefix};
use betterlite_models::identity::openrouter::normalize_openrouter_prefix;
use serde_json::to_value;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "check", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ExplicitProvider {
        model: String,
        custom_llm_provider: Option<String>,
        configured_model_names: Vec<String>,
        catalog: serde_json::Value,
    },
    Cohere {
        model: String,
        custom_llm_provider: Option<String>,
        catalog: serde_json::Value,
    },
    AnthropicText {
        model: String,
        custom_llm_provider: Option<String>,
    },
    AzureClaude {
        model: String,
    },
    Openrouter {
        model: String,
        custom_llm_provider: Option<String>,
    },
    Endpoint {
        endpoint: String,
        api_base: String,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ExplicitProvider {
                model,
                custom_llm_provider,
                configured_model_names,
                catalog,
            } => run_explicit_provider(
                model,
                custom_llm_provider.as_deref(),
                configured_model_names,
                catalog,
            ),
            Input::Cohere {
                model,
                custom_llm_provider,
                catalog,
            } => {
                let catalog = parse_catalog(catalog)?;
                to_json(&cohere_chat_provider(
                    model,
                    custom_llm_provider.as_deref(),
                    &catalog,
                ))
            }
            Input::AnthropicText {
                model,
                custom_llm_provider,
            } => to_json(&anthropic_text_provider(
                model,
                custom_llm_provider.as_deref(),
            )),
            Input::AzureClaude { model } => to_json(&is_azure_claude_model(model)),
            Input::Openrouter {
                model,
                custom_llm_provider,
            } => to_json(&normalize_openrouter_prefix(
                model,
                custom_llm_provider.as_deref(),
            )),
            Input::Endpoint { endpoint, api_base } => {
                to_json(&endpoint_matches_api_base(endpoint, api_base))
            }
        }
    }
}

fn run_explicit_provider(
    model: &str,
    custom_llm_provider: Option<&str>,
    configured_model_names: &[String],
    catalog_json: &serde_json::Value,
) -> HelperResult {
    let catalog = parse_catalog(catalog_json)?;
    let configured_names = configured_model_names.iter().cloned().collect();
    let classification = classify_prefix(&PrefixClassificationInput {
        model,
        custom_llm_provider,
        configured_model_names: &configured_names,
        catalog: &catalog,
    });
    to_json(&classification)
}

fn parse_catalog(source: &serde_json::Value) -> Result<ModelCatalog, Box<HelperError>> {
    let bytes = source.to_string();
    ModelCatalog::parse(bytes.as_bytes(), UnknownFields::Reject).map_err(|error| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

fn to_json<T>(value: &T) -> HelperResult
where
    T: serde::Serialize,
{
    match to_value(value) {
        Ok(json) => Ok(json),
        Err(error) => Err(Box::new(HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })),
    }
}
