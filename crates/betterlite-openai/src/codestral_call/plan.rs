use super::error::CodestralCallError;
use super::log::{CallLogger, PreCall};
use crate::base_helpers::provider_config;
use crate::codestral_records::options::CodestralSettings;
use crate::openai_base_config::Headers;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::custom::{CustomPromptTemplate, custom_prompt};
use betterlite_prompts::factory::{Output, PromptFactory, prompt_factory};
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::time::Duration;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    Async,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Wrapper {
    Codestral,
    TextCompletionCodestral,
}
pub struct CodestralRequest<'call> {
    pub api_key: Option<&'call SecretString>,
    pub headers: &'call Headers,
    pub model: &'call str,
    pub messages: &'call [Message],
    pub api_base: Option<&'call str>,
    pub custom_prompts: &'call BTreeMap<String, CustomPromptTemplate>,
    pub optional_params: &'call Map<String, Value>,
    pub settings: &'call CodestralSettings,
    pub mode: Mode,
    pub timeout: Duration,
}
pub struct Plan {
    pub mode: Mode,
    pub headers: Headers,
    pub url: String,
    pub body: Map<String, Value>,
    pub wrapper: Option<Wrapper>,
    pub timeout: Option<Duration>,
    pub prompt: String,
    pub messages: Vec<Value>,
    pub api_key: SecretString,
}
pub fn validate_environment(
    api_key: Option<&SecretString>,
    user_headers: &Headers,
) -> Result<(Headers, SecretString), CodestralCallError> {
    let key = api_key.ok_or(CodestralCallError::MissingApiKey)?;
    let mut headers = Headers::from([
        ("content-type".to_owned(), "application/json".to_owned()),
        (
            "Authorization".to_owned(),
            format!("Bearer {}", key.expose_secret()),
        ),
    ]);
    headers.extend(user_headers.clone());
    Ok((headers, key.clone()))
}
pub async fn prepare(
    request: &CodestralRequest<'_>,
    prompts: &PromptFactory,
    now: DateTime<Utc>,
    log: &mut (dyn CallLogger + Send),
) -> Result<Plan, CodestralCallError> {
    let (headers, api_key) = validate_environment(request.api_key, request.headers)?;
    let mut options = request.optional_params.clone();
    let url = resolve_url(&mut options, request.api_base);
    let prompt = build_prompt(request, prompts, now).await?;
    let (body, streaming) = build_body(request, &options, &prompt)?;
    let (wrapper, timeout) = match (request.mode, streaming) {
        (Mode::Sync, false) => (None, None),
        (Mode::Sync, true) => (Some(Wrapper::Codestral), None),
        (Mode::Async, true) => (Some(Wrapper::TextCompletionCodestral), None),
        (Mode::Async, false) => (None, Some(request.timeout)),
    };
    let mut plan = Plan {
        mode: request.mode,
        headers,
        url,
        body,
        wrapper,
        timeout,
        prompt,
        messages: serialized_messages(request.messages)?,
        api_key,
    };
    log_pre_call(&plan, request.api_key, log);
    if request.mode == Mode::Async && streaming {
        plan.body.insert("stream".to_owned(), Value::Bool(true));
    }
    Ok(plan)
}
fn resolve_url(options: &mut Map<String, Value>, api_base: Option<&str>) -> String {
    if matches!(options.remove("custom_endpoint"), Some(Value::Bool(true))) {
        return api_base.unwrap_or_default().to_owned();
    }
    api_base
        .filter(|base| !base.is_empty())
        .unwrap_or("https://codestral.mistral.ai/v1/fim/completions")
        .to_owned()
}
async fn build_prompt(
    request: &CodestralRequest<'_>,
    prompts: &PromptFactory,
    now: DateTime<Utc>,
) -> Result<String, CodestralCallError> {
    if let Some(custom) = request.custom_prompts.get(request.model) {
        let template = CustomPromptTemplate {
            roles: custom.roles.clone(),
            initial_prompt_value: custom.initial_prompt_value.clone(),
            final_prompt_value: custom.final_prompt_value.clone(),
            bos_token: String::new(),
            eos_token: String::new(),
        };
        return custom_prompt(&template, request.messages).map_err(CodestralCallError::from);
    }
    match prompt_factory(prompts, request.model, request.messages, None, now).await? {
        Output::Text(text) => Ok(text),
        Output::None | Output::Ollama(_) => Err(CodestralCallError::PromptNotText),
    }
}
fn build_body(
    request: &CodestralRequest<'_>,
    options: &Map<String, Value>,
    prompt: &str,
) -> Result<(Map<String, Value>, bool), CodestralCallError> {
    let mut merged = options.clone();
    for (name, value) in provider_config(request.settings)? {
        merged.entry(name).or_insert(value);
    }
    let streaming = matches!(merged.remove("stream"), Some(Value::Bool(true)));
    let mut body = Map::new();
    body.insert("model".to_owned(), Value::String(request.model.to_owned()));
    body.insert("prompt".to_owned(), Value::String(prompt.to_owned()));
    body.extend(merged);
    Ok((body, streaming))
}
fn serialized_messages(messages: &[Message]) -> Result<Vec<Value>, CodestralCallError> {
    messages
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| CodestralCallError::Encode { source })
}
fn log_pre_call(plan: &Plan, api_key: Option<&SecretString>, log: &mut (dyn CallLogger + Send)) {
    log.pre_call(PreCall {
        input: &plan.prompt,
        api_key,
        body: &plan.body,
        headers: &plan.headers,
        api_base: &plan.url,
        acompletion: plan.mode == Mode::Async,
    });
}
