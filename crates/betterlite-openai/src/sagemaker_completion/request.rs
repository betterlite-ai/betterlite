use std::collections::BTreeMap;

use betterlite_prompts::builtin::Message;
use betterlite_prompts::error::FactoryError;
use betterlite_prompts::factory::custom::{CustomPromptTemplate, RoleTemplate, custom_prompt};
use betterlite_prompts::factory::{Output, PromptFactory, prompt_factory};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Value};
use thiserror::Error;

use crate::openai_base_config::Headers;
use crate::sagemaker_completion::params::Options;

const LLAMA_2_CHAT_MODEL: &str = "meta-llama/Llama-2-7b-chat-hf";
const LLAMA_2_MODEL: &str = "meta-llama/Llama-2-7b";
const CONTENT_TYPE: &str = "Content-Type";
const JSON_CONTENT_TYPE: &str = "application/json";

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CustomPromptEntry {
    #[serde(default)]
    pub roles: BTreeMap<String, RoleTemplate>,
    #[serde(default)]
    pub initial_prompt_value: Option<String>,
    #[serde(default)]
    pub final_prompt_value: Option<String>,
}

#[derive(Debug, Error)]
pub enum RequestError {
    #[error(transparent)]
    Prompt(#[from] FactoryError),
    #[error("the {model} model has no chat prompt template")]
    NoTemplate { model: String },
}

pub struct RequestInput<'request> {
    pub model: &'request str,
    pub messages: &'request [Message],
    pub optional_params: &'request Options,
    pub custom_prompt_dict: &'request BTreeMap<String, CustomPromptEntry>,
    pub global_custom_prompt_dict: &'request BTreeMap<String, CustomPromptEntry>,
    pub hf_model_name: Option<&'request str>,
    pub prompts: &'request PromptFactory,
    pub now: DateTime<Utc>,
}

pub async fn transform_request(
    input: &RequestInput<'_>,
) -> Result<IndexMap<String, Value>, RequestError> {
    let prompt = transform_prompt(input).await?;
    let mut inference_params = input.optional_params.clone();
    let stream = inference_params.remove("stream");
    let mut data = IndexMap::new();
    let parameters: Map<String, Value> = inference_params.into_iter().collect();
    data.insert(String::from("parameters"), Value::Object(parameters));
    // a stream value other than exactly true is dropped from the body, as the pinned pop does
    if stream == Some(Value::Bool(true)) {
        data.insert(String::from("stream"), Value::Bool(true));
    }
    data.insert(String::from("inputs"), Value::String(prompt));
    Ok(data)
}

// LiteLLM reassigns its headers argument before reading it, so a caller's headers never reach the request
#[must_use]
pub fn validate_environment() -> Headers {
    let mut headers = Headers::new();
    headers.insert(String::from(CONTENT_TYPE), String::from(JSON_CONTENT_TYPE));
    headers
}

async fn transform_prompt(input: &RequestInput<'_>) -> Result<String, RequestError> {
    let dict = if input.custom_prompt_dict.is_empty() {
        input.global_custom_prompt_dict
    } else {
        input.custom_prompt_dict
    };
    if let Some(entry) = dict.get(input.model) {
        return render_prompt(entry, input.messages);
    }
    if let Some(name) = input.hf_model_name
        && let Some(entry) = dict.get(name)
    {
        return render_prompt(entry, input.messages);
    }
    let template_model = input
        .hf_model_name
        .map_or_else(|| llama_2_template(input.model), str::to_owned);
    match prompt_factory(
        input.prompts,
        &template_model,
        input.messages,
        None,
        input.now,
    )
    .await?
    {
        Output::Text(prompt) => Ok(prompt),
        Output::Ollama(_) | Output::None => Err(RequestError::NoTemplate {
            model: template_model,
        }),
    }
}

fn render_prompt(entry: &CustomPromptEntry, messages: &[Message]) -> Result<String, RequestError> {
    let template = CustomPromptTemplate {
        roles: entry.roles.clone(),
        initial_prompt_value: entry.initial_prompt_value.clone().unwrap_or_default(),
        final_prompt_value: entry.final_prompt_value.clone().unwrap_or_default(),
        bos_token: String::new(),
        eos_token: String::new(),
    };
    custom_prompt(&template, messages).map_err(RequestError::from)
}

fn llama_2_template(model: &str) -> String {
    let lower = model.to_lowercase();
    if !lower.contains("llama-2") {
        return model.to_owned();
    }
    if lower.contains("chat") {
        return String::from(LLAMA_2_CHAT_MODEL);
    }
    String::from(LLAMA_2_MODEL)
}

mod tests;
