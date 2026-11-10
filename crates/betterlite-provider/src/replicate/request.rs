use std::collections::BTreeMap;

use betterlite_logging::analytics::python;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::custom::{CustomPromptTemplate, custom_prompt};
use betterlite_prompts::factory::{Output, PromptFactory, prompt_factory};
use betterlite_prompts::message_content::{Message as ContentMessage, convert_content_list_to_str};
use chrono::{DateTime, Utc};
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::error::{ReplicateError, ReplicateFailure};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicateBody {
    pub input: Object,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

// the typed form of ReplicateConfig(system_prompt=..., max_new_tokens=..., ...); nothing in betterlite sets it
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplicateDefaults {
    pub system_prompt: Option<String>,
    pub max_new_tokens: Option<i64>,
    pub min_new_tokens: Option<i64>,
    pub temperature: Option<i64>,
    pub top_p: Option<i64>,
    pub top_k: Option<i64>,
    pub stop_sequences: Option<String>,
    pub seed: Option<i64>,
    pub debug: Option<bool>,
}

pub struct RequestInput<'request> {
    pub model: &'request str,
    pub messages: &'request mut Vec<Message>,
    pub optional_params: &'request mut Object,
    pub defaults: &'request ReplicateDefaults,
    pub custom_prompts: &'request BTreeMap<String, CustomPromptTemplate>,
}

pub struct RequestContext<'deps> {
    pub prompts: &'deps PromptFactory,
    pub now: DateTime<Utc>,
}

fn insert(optional_params: &mut Object, name: &str, value: Option<Json>) {
    if let Some(found) = value {
        optional_params.entry(String::from(name)).or_insert(found);
    }
}

// config values only fill optional_params where the call has no key, so a call value always wins
fn apply_defaults(defaults: &ReplicateDefaults, optional_params: &mut Object) {
    let number = |raw: Option<i64>| raw.map(|value| Json::Number(Number::from(value)));
    insert(
        optional_params,
        "system_prompt",
        defaults
            .system_prompt
            .as_ref()
            .map(|value| Json::Text(value.clone())),
    );
    insert(
        optional_params,
        "max_new_tokens",
        number(defaults.max_new_tokens),
    );
    insert(
        optional_params,
        "min_new_tokens",
        number(defaults.min_new_tokens),
    );
    insert(optional_params, "temperature", number(defaults.temperature));
    insert(optional_params, "top_p", number(defaults.top_p));
    insert(optional_params, "top_k", number(defaults.top_k));
    insert(
        optional_params,
        "stop_sequences",
        defaults
            .stop_sequences
            .as_ref()
            .map(|value| Json::Text(value.clone())),
    );
    insert(optional_params, "seed", number(defaults.seed));
    insert(optional_params, "debug", defaults.debug.map(Json::Bool));
}

fn take_supports_system_prompt(optional_params: &mut Object) -> Result<bool, ReplicateFailure> {
    match optional_params.shift_remove("supports_system_prompt") {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(value)) => Ok(value),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(ReplicateFailure::SupportsSystemPrompt)
        }
    }
}

fn as_content_message(message: &Message) -> Result<ContentMessage, ReplicateFailure> {
    let bytes =
        serde_json::to_vec(message).map_err(|source| ReplicateFailure::Messages { source })?;
    serde_json::from_slice(&bytes).map_err(|source| ReplicateFailure::Messages { source })
}

fn take_system_prompt(
    messages: &mut Vec<Message>,
    supports_system_prompt: bool,
) -> Result<Option<String>, ReplicateFailure> {
    if !supports_system_prompt {
        return Ok(None);
    }
    let Some(index) = messages.iter().position(|message| message.role == "system") else {
        return Ok(None);
    };
    let message = messages.remove(index);
    let prompt_message = as_content_message(&message)?;
    Ok(Some(convert_content_list_to_str(&prompt_message)))
}

async fn render_prompt(
    model: &str,
    messages: &[Message],
    custom_prompts: &BTreeMap<String, CustomPromptTemplate>,
    context: &RequestContext<'_>,
) -> Result<String, ReplicateFailure> {
    if let Some(template) = custom_prompts.get(model) {
        return Ok(custom_prompt(template, messages)?);
    }
    match prompt_factory(context.prompts, model, messages, None, context.now).await? {
        Output::Text(text) => Ok(text),
        Output::Ollama(prompt) => {
            let rendered =
                python::repr(&prompt).map_err(|source| ReplicateFailure::Format { source })?;
            Err(ReplicateFailure::Api(Box::new(ReplicateError {
                status_code: 400,
                message: format!("LiteLLM Error - prompt is not a string - {rendered}"),
                headers: Some(HeaderMap::new()),
            })))
        }
        Output::None => Err(ReplicateFailure::Api(Box::new(ReplicateError {
            status_code: 400,
            message: String::from("LiteLLM Error - prompt is not a string - None"),
            headers: Some(HeaderMap::new()),
        }))),
    }
}

fn input_data(prompt: String, system_prompt: Option<String>, optional_params: &Object) -> Object {
    let mut input = Object::new();
    input.insert(String::from("prompt"), Json::Text(prompt));
    if let Some(text) = system_prompt {
        input.insert(String::from("system_prompt"), Json::Text(text));
    }
    input.extend(optional_params.clone());
    input
}

pub async fn transform_request(
    input: RequestInput<'_>,
    context: &RequestContext<'_>,
) -> Result<ReplicateBody, ReplicateFailure> {
    let RequestInput {
        model,
        messages,
        optional_params,
        defaults,
        custom_prompts,
    } = input;
    apply_defaults(defaults, optional_params);
    let supports_system_prompt = take_supports_system_prompt(optional_params)?;
    let system_prompt = take_system_prompt(messages, supports_system_prompt)?;
    let prompt = render_prompt(model, messages, custom_prompts, context).await?;
    Ok(ReplicateBody {
        input: input_data(prompt, system_prompt, optional_params),
        // LiteLLM adds the version only when the id, already split on colons, holds a colon; that never happens
        version: None,
    })
}

#[must_use]
pub fn model_to_version_id(model: &str) -> &str {
    let Some(version_id) = model.split(':').nth(1) else {
        return model;
    };
    version_id
}

#[must_use]
pub fn get_complete_url(model: &str) -> String {
    let version_id = model_to_version_id(model);
    if version_id.contains("deployments") {
        let deployment = version_id.replace("deployments/", "");
        format!("https://api.replicate.com/v1/deployments/{deployment}/predictions")
    } else {
        format!("https://api.replicate.com/v1/models/{version_id}/predictions")
    }
}

pub fn validate_environment(
    _headers: &HeaderMap,
    api_key: Option<&str>,
) -> Result<HeaderMap, ReplicateFailure> {
    let key = api_key.map_or_else(|| String::from("None"), str::to_owned);
    let mut authorization = HeaderValue::try_from(format!("Token {key}"))
        .map_err(|source| ReplicateFailure::Header { source })?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, authorization);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(headers)
}

mod tests;
