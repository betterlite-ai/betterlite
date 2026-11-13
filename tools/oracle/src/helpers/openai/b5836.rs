use std::collections::BTreeMap;

use betterlite_openai::cloudflare_chat::error::get_error_class;
use betterlite_openai::cloudflare_chat::{complete_url, validate_environment};
use betterlite_openai::cloudflare_decisions::CloudflareDecisionsConfig;
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig as _;
use betterlite_openai::error::ProviderError;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    chat_urls: Vec<ChatUrlCall>,
    #[serde(default)]
    chat_headers: Vec<ChatHeadersCall>,
    #[serde(default)]
    chat_errors: Vec<ChatErrorCall>,
    #[serde(default)]
    decisions_defaults: Vec<DefaultsCall>,
    #[serde(default)]
    decisions_models: Vec<String>,
    #[serde(default)]
    decisions_urls: Vec<DecisionsUrlCall>,
    #[serde(default)]
    unwrap: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatUrlCall {
    api_base: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatHeadersCall {
    api_key: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatErrorCall {
    message: String,
    status_code: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultsCall {
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionsUrlCall {
    api_base: String,
    model: String,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let decisions = CloudflareDecisionsConfig::new(None);
        Ok(json!({
            "chat_urls": input.chat_urls.iter().map(chat_url).collect::<Vec<_>>(),
            "chat_headers": input.chat_headers.iter().map(chat_headers).collect::<Vec<_>>(),
            "chat_errors": input.chat_errors.iter().map(chat_error).collect::<Vec<_>>(),
            "decisions_defaults": input.decisions_defaults.iter().map(decisions_defaults).collect::<Vec<_>>(),
            "decisions_models": input.decisions_models.iter().map(|model| decisions_model(&decisions, model)).collect::<Vec<_>>(),
            "decisions_urls": input.decisions_urls.iter().map(|call| decisions_url(&decisions, call)).collect::<Vec<_>>(),
            "unwrap": input.unwrap.iter().map(|payload| unwrap(&decisions, payload)).collect::<Vec<_>>(),
        }))
    }
}

fn chat_url(call: &ChatUrlCall) -> Value {
    let account_id = call.env.get("CLOUDFLARE_ACCOUNT_ID").map(String::as_str);
    match complete_url(call.api_base.as_deref(), account_id) {
        Ok(url) => json!({"url": url}),
        Err(error) => json!({"error": error.to_string()}),
    }
}

fn chat_headers(call: &ChatHeadersCall) -> Value {
    let key = call.api_key.as_deref().map(SecretString::from);
    match validate_environment(call.headers.clone(), key.as_ref()) {
        Ok(headers) => json!({"headers": headers}),
        Err(error) => json!({"error": error.to_string()}),
    }
}

fn chat_error(call: &ChatErrorCall) -> Value {
    let error = ProviderError::from(get_error_class(
        &call.message,
        call.status_code,
        &call.headers,
    ));
    json!({
        "class": "CloudflareError",
        "status_code": error.status_code,
        "message": error.message,
        "request_method": error.request.method,
        "request_url": error.request.url,
        "response_status": error.response.status_code,
        "headers": error.headers,
    })
}

fn decisions_defaults(call: &DefaultsCall) -> Value {
    let config = CloudflareDecisionsConfig::new(call.env.get("CLOUDFLARE_ACCOUNT_ID").cloned());
    json!({
        "api_key_env": CloudflareDecisionsConfig::API_KEY_ENV,
        "api_base_env": CloudflareDecisionsConfig::API_BASE_ENV,
        "default_base": config.get_default_api_base(),
        "missing_base_message": config.missing_api_base_message("cloudflare"),
    })
}

fn decisions_model(config: &CloudflareDecisionsConfig, model: &str) -> Value {
    json!({
        "canonical": config.canonical_model(model),
        "request": config.request_model(model),
    })
}

fn decisions_url(config: &CloudflareDecisionsConfig, call: &DecisionsUrlCall) -> Value {
    json!({
        "url": config.get_complete_url(&call.api_base, &config.canonical_model(&call.model)),
    })
}

fn unwrap(config: &CloudflareDecisionsConfig, payload: &Value) -> Value {
    let Ok(bytes) = serde_json::to_vec(payload) else {
        return Value::Null;
    };
    serde_json::from_slice(config.unwrap_response(&bytes)).unwrap_or(Value::Null)
}
