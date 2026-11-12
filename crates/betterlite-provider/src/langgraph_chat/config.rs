use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use super::error::LangGraphFailure;

pub const API_BASE_ENV: &str = "LANGGRAPH_API_BASE";
pub const API_KEY_ENV: &str = "LANGGRAPH_API_KEY";
pub const DEFAULT_API_BASE: &str = "http://localhost:2024";
pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["stream"];
// the endpoint carries the stream choice, so the body has no stream parameter
pub const STREAM_PARAM_IN_BODY: bool = false;
// LangGraph streams natively
pub const SHOULD_FAKE_STREAM: bool = false;

#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub api_base: String,
    pub api_key: Option<SecretString>,
}

// LiteLLM reads an empty base or key as unset, in the arguments and in the environment alike
#[must_use]
pub fn provider_info(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    env_api_base: Option<&str>,
    env_api_key: Option<&SecretString>,
) -> ProviderInfo {
    let base = [api_base, env_api_base]
        .into_iter()
        .flatten()
        .find(|base| !base.is_empty())
        .unwrap_or(DEFAULT_API_BASE);
    let key = [api_key, env_api_key]
        .into_iter()
        .flatten()
        .find(|key| !key.expose_secret().is_empty());
    ProviderInfo {
        api_base: base.to_owned(),
        api_key: key.cloned(),
    }
}

pub fn get_complete_url(api_base: Option<&str>, stream: bool) -> Result<String, LangGraphFailure> {
    let base = api_base
        .ok_or(LangGraphFailure::MissingApiBase)?
        .trim_end_matches('/');
    let endpoint = if stream { "runs/stream" } else { "runs/wait" };
    Ok(format!("{base}/{endpoint}"))
}

pub fn validate_environment(
    headers: &mut BTreeMap<String, String>,
    api_key: Option<&SecretString>,
) {
    headers.insert(
        String::from("Content-Type"),
        String::from("application/json"),
    );
    if let Some(key) = api_key.filter(|key| !key.expose_secret().is_empty()) {
        headers.insert(
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        );
    }
}
