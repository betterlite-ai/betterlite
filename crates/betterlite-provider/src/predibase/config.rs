use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use super::error::{PredibaseError, PredibaseSetupError, get_error_class};

pub const DEFAULT_API_BASE: &str = "https://serving.app.predibase.com";

// TGI rejects a temperature of zero, so LiteLLM sends the smallest positive value it names
const MINIMUM_TEMPERATURE: f64 = 0.01;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredibaseConfig {
    pub details: bool,
    pub max_new_tokens: i64,
    pub return_full_text: bool,
}

impl PredibaseConfig {
    #[must_use]
    pub fn get_config(&self) -> Object {
        let mut config = Object::new();
        config.insert(String::from("details"), Json::Bool(self.details));
        config.insert(
            String::from("max_new_tokens"),
            Json::Number(Number::from(self.max_new_tokens)),
        );
        config.insert(
            String::from("return_full_text"),
            Json::Bool(self.return_full_text),
        );
        config
    }
}

#[must_use]
pub const fn get_supported_openai_params() -> [&'static str; 8] {
    [
        "stream",
        "temperature",
        "max_completion_tokens",
        "max_tokens",
        "top_p",
        "stop",
        "n",
        "response_format",
    ]
}

pub fn map_openai_params(non_default: &Object, options: &mut Object) {
    for (name, value) in non_default {
        match name.as_str() {
            "temperature" => {
                let zero =
                    matches!(value, Json::Number(number) if number.as_f64() == Some(0.0_f64));
                let floored = if zero {
                    Number::from_f64(MINIMUM_TEMPERATURE).map_or(Json::Null, Json::Number)
                } else {
                    value.clone()
                };
                options.insert(name.clone(), floored);
            }
            "top_p" | "stream" | "stop" | "response_format" => {
                options.insert(name.clone(), value.clone());
            }
            "n" => {
                options.insert(String::from("best_of"), value.clone());
                options.insert(String::from("do_sample"), Json::Bool(true));
            }
            "max_tokens" | "max_completion_tokens" => {
                let zero =
                    matches!(value, Json::Number(number) if number.as_f64() == Some(0.0_f64));
                let limit = if zero {
                    Json::Number(Number::from(1_i64))
                } else {
                    value.clone()
                };
                options.insert(String::from("max_new_tokens"), limit);
            }
            "echo" => {
                options.insert(String::from("decoder_input_details"), Json::Bool(true));
            }
            _ => {}
        }
    }
}

pub fn get_complete_url(
    tenant_id: Option<&str>,
    api_base: Option<&str>,
    model: &str,
    options: &Object,
    stream: Option<bool>,
) -> Result<String, PredibaseSetupError> {
    let tenant = tenant_id
        .filter(|tenant| !tenant.is_empty())
        .ok_or(PredibaseSetupError::MissingTenantId)?;
    let base = api_base
        .filter(|base| !base.is_empty())
        .unwrap_or(DEFAULT_API_BASE);
    let streams = stream.unwrap_or_else(|| {
        options
            .get("stream")
            .is_some_and(|value| matches!(value, Json::Bool(true)))
    });
    let suffix = if streams {
        "/generate_stream"
    } else {
        "/generate"
    };
    Ok(format!(
        "{base}/{tenant}/deployments/v2/llms/{model}{suffix}"
    ))
}

pub fn validate_environment(
    api_key: Option<&SecretString>,
    headers: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, PredibaseSetupError> {
    let Some(key) = api_key else {
        return Err(PredibaseSetupError::MissingApiKey);
    };
    let mut merged = BTreeMap::from([
        (
            String::from("content-type"),
            String::from("application/json"),
        ),
        (
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        ),
    ]);
    merged.extend(
        headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    Ok(merged)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallFailure<'message> {
    // the provider answered with this status and body
    Response { status: u16, body: &'message str },
    // an already typed client error such as LiteLLM's Timeout, which it re-raises unchanged
    Rethrown,
    // the client failed before a response for another reason
    Transport { message: &'message str },
}

#[must_use]
pub fn call_error(failure: CallFailure<'_>, streaming: bool) -> Option<PredibaseError> {
    let headers = BTreeMap::new();
    match failure {
        CallFailure::Rethrown => None,
        CallFailure::Transport { message } => {
            (!streaming).then(|| get_error_class(message, 500, &headers))
        }
        CallFailure::Response { status, body } => {
            if streaming {
                return (status != 200).then(|| get_error_class(body, status, &headers));
            }
            // httpx raise_for_status accepts every 2xx, so only an HTTP error status raises here
            if (200..300).contains(&status) {
                return None;
            }
            Some(get_error_class(
                &format!("HTTPStatusError - received status_code={status}, error_message={body}"),
                status,
                &headers,
            ))
        }
    }
}
