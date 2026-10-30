use std::time::{Duration, TryFromFloatSecsError};

use betterlite_config::types::ModelDeployment;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use url::Url;

const OPENAI_API_BASE: &str = "https://api.openai.com/v1";
// LiteLLM's DEFAULT_REQUEST_TIMEOUT_SECONDS
const REQUEST_TIMEOUT_SECONDS: f64 = 6000.0;

#[derive(Debug, Error)]
pub enum DeploymentError {
    #[error("model_list entry {model_name} has an api_base that is not a URL: {source}")]
    ApiBase {
        model_name: String,
        #[source]
        source: url::ParseError,
    },
    #[error(
        "model_list entry {model_name} has a timeout of {seconds} seconds, which no wait can last"
    )]
    Timeout {
        model_name: String,
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provider {
    OpenAi { model: String },
    Unsupported { model: String },
}

// litellm_settings and environment values an OpenAI deployment falls back to, in LiteLLM's order
#[derive(Debug, Clone, Default)]
pub struct OpenAiDefaults {
    pub request_timeout: Option<f64>,
    pub api_bases: Vec<String>,
    pub api_keys: Vec<SecretString>,
    pub organization: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChatDeployment {
    pub model_group: String,
    pub litellm_model: String,
    pub provider: Provider,
    pub api_base: String,
    pub endpoint: Url,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
    pub timeout: Duration,
    pub timeout_seconds: f64,
    pub model_id: String,
}

impl ChatDeployment {
    pub fn resolve(
        deployment: &ModelDeployment,
        defaults: &OpenAiDefaults,
    ) -> Result<Self, DeploymentError> {
        let params = &deployment.litellm_params;
        let model_name = &deployment.model_name;
        let litellm_model = params.model.clone().unwrap_or_default();
        let provider = litellm_model.strip_prefix("openai/").map_or_else(
            || Provider::Unsupported {
                model: litellm_model.clone(),
            },
            |model| Provider::OpenAi {
                model: model.to_owned(),
            },
        );
        let api_base = params
            .api_base
            .iter()
            .chain(&defaults.api_bases)
            .find(|base| !base.is_empty())
            .map_or(OPENAI_API_BASE, String::as_str);
        let (seconds, wait) = timeout(params.timeout, defaults, model_name)?;
        Ok(Self {
            timeout: wait,
            model_group: model_name.clone(),
            api_base: api_base.split('?').next().unwrap_or_default().to_owned(),
            endpoint: endpoint(api_base, model_name)?,
            api_key: params
                .api_key
                .iter()
                .chain(&defaults.api_keys)
                .next()
                .cloned(),
            organization: defaults.organization.clone(),
            timeout_seconds: seconds,
            model_id: model_id(deployment),
            litellm_model,
            provider,
        })
    }

    pub fn endpoint_for(&self, resource: &str) -> Result<Url, DeploymentError> {
        let mut url = endpoint(&self.api_base, &self.model_group)?;
        let path = format!(
            "{}/{}",
            url.path().trim_end_matches("/chat/completions"),
            resource
        );
        url.set_path(&path);
        Ok(url)
    }
}

fn timeout(
    seconds: Option<f64>,
    defaults: &OpenAiDefaults,
    model_name: &str,
) -> Result<(f64, Duration), DeploymentError> {
    let resolved = seconds
        .or(defaults.request_timeout)
        .unwrap_or(REQUEST_TIMEOUT_SECONDS);
    let wait =
        Duration::try_from_secs_f64(resolved).map_err(|source| DeploymentError::Timeout {
            model_name: model_name.to_owned(),
            seconds: resolved,
            source,
        })?;
    Ok((resolved, wait))
}

fn endpoint(api_base: &str, model_name: &str) -> Result<Url, DeploymentError> {
    let mut url = Url::parse(api_base).map_err(|source| DeploymentError::ApiBase {
        model_name: model_name.to_owned(),
        source,
    })?;
    let path = format!("{}/chat/completions", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url)
}

// LiteLLM's generate_model_id hashes the model group and each litellm_params key and value in order
fn model_id(deployment: &ModelDeployment) -> String {
    if let Some(id) = &deployment.model_info.id {
        return id.clone();
    }
    let params = &deployment.litellm_params;
    let numbers = [
        ("timeout", params.timeout.map(|value| value.to_string())),
        (
            "stream_timeout",
            params.stream_timeout.map(|value| value.to_string()),
        ),
        (
            "max_retries",
            params.max_retries.map(|value| value.to_string()),
        ),
        ("rpm", params.rpm.map(|value| value.to_string())),
        ("tpm", params.tpm.map(|value| value.to_string())),
        ("weight", params.weight.map(|value| value.to_string())),
    ];
    let mut hasher = Sha256::new();
    hasher.update(deployment.model_name.as_bytes());
    let texts = [
        ("model", params.model.as_deref()),
        ("api_base", params.api_base.as_deref()),
        (
            "api_key",
            params.api_key.as_ref().map(ExposeSecret::expose_secret),
        ),
        ("api_version", params.api_version.as_deref()),
    ];
    let numbered = numbers
        .iter()
        .map(|(name, value)| (*name, value.as_deref()));
    for (name, value) in texts.into_iter().chain(numbered) {
        if let Some(text) = value {
            hasher.update(name.as_bytes());
            hasher.update(text.as_bytes());
        }
    }
    hex::encode(hasher.finalize())
}

mod tests;
