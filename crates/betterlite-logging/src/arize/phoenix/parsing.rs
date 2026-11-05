use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
use thiserror::Error;

use crate::arize::phoenix::records::{PromptMetadata, PromptTemplate, PromptVersionData};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParsingError {
    #[error("invocation_parameters.{provider} must be an object")]
    ProviderParameters { provider: String },
    #[error("invocation_parameters temperature must be a number")]
    Temperature,
    #[error("invocation_parameters max_tokens must be an integer")]
    MaxTokens,
}

#[must_use]
pub fn named_provider(
    invocation_parameters: &IndexMap<String, Json>,
) -> Option<(&'static str, &Json)> {
    ["openai", "anthropic"]
        .into_iter()
        .find_map(|provider| Some((provider, invocation_parameters.get(provider)?)))
}

fn selected_provider_parameters(
    invocation_parameters: &IndexMap<String, Json>,
) -> Result<Option<&IndexMap<String, Json>>, ParsingError> {
    match named_provider(invocation_parameters) {
        Some((_, Json::Object(parameters))) => Ok(Some(parameters)),
        Some((
            provider,
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
        )) => Err(ParsingError::ProviderParameters {
            provider: provider.to_owned(),
        }),
        None => Ok(invocation_parameters
            .values()
            .find_map(|value| match value {
                Json::Object(parameters) => Some(parameters),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    None
                }
            })),
    }
}

fn optional_number(
    parameters: Option<&IndexMap<String, Json>>,
    name: &str,
    error: ParsingError,
) -> Result<Option<Number>, ParsingError> {
    match parameters.and_then(|found| found.get(name)) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => Ok(Some(number.clone())),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => Err(error),
    }
}

pub fn parse_prompt_data(
    data: PromptVersionData,
    prompt_version_id: &str,
) -> Result<PromptTemplate, ParsingError> {
    let provider_parameters = selected_provider_parameters(&data.invocation_parameters)?;
    let temperature = optional_number(
        provider_parameters,
        "temperature",
        ParsingError::Temperature,
    )?;
    let max_tokens = optional_number(provider_parameters, "max_tokens", ParsingError::MaxTokens)?;
    if max_tokens
        .as_ref()
        .is_some_and(|tokens| !tokens.is_i64() && !tokens.is_u64())
    {
        return Err(ParsingError::MaxTokens);
    }
    Ok(PromptTemplate {
        template_id: prompt_version_id.to_owned(),
        messages: data.template.messages,
        metadata: PromptMetadata {
            model_name: data.model_name,
            model_provider: data.model_provider,
            description: data.description,
            template_type: data.template_type,
            template_format: data.template_format,
            invocation_parameters: data.invocation_parameters,
            temperature,
            max_tokens,
        },
    })
}

mod tests;
