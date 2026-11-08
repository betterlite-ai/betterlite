use std::time::{Duration, TryFromFloatSecsError};

use betterlite_config::types::ModelDeployment;
use betterlite_http::url_policy::base::ConfiguredBase;
use betterlite_http::url_policy::error::UrlPolicyError;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use url::Url;

const OPENAI_API_BASE: &str = "https://api.openai.com/v1";
const API_BASE_KEY: &str = "litellm_params.api_base";
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
    #[error("model_list entry {model_name} has an api_base that cannot be used: {source}")]
    ConfiguredBase {
        model_name: String,
        #[source]
        source: UrlPolicyError,
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
    // the Basic header HTTPX would build from userinfo in api_base, which is stripped from every stored URL
    pub authorization: Option<SecretString>,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
    pub timeout: Duration,
    pub timeout_seconds: f64,
    pub model_id: String,
    pub weight: u32,
    pub rpm: u32,
    pub tpm: u32,
}

impl ChatDeployment {
    pub fn resolve(
        deployment: &ModelDeployment,
        defaults: &OpenAiDefaults,
    ) -> Result<Self, DeploymentError> {
        let params = &deployment.litellm_params;
        let model_name = &deployment.model_name;
        let litellm_model = params.model.clone().unwrap_or_default();
        let provider = provider_of(&litellm_model);
        let api_base = params
            .api_base
            .iter()
            .chain(&defaults.api_bases)
            .find(|base| !base.is_empty())
            .map_or(OPENAI_API_BASE, String::as_str);
        let (seconds, wait) = timeout(params.timeout, defaults, model_name)?;
        let configured = configured_base(api_base, model_name)?;
        Ok(Self {
            timeout: wait,
            model_group: model_name.clone(),
            api_base: stored_api_base(&configured, api_base),
            endpoint: endpoint(configured.url),
            authorization: configured.authorization,
            api_key: params
                .api_key
                .iter()
                .chain(&defaults.api_keys)
                .next()
                .cloned(),
            organization: defaults.organization.clone(),
            timeout_seconds: seconds,
            model_id: model_id(deployment),
            weight: params.weight.unwrap_or_default(),
            rpm: params.rpm.unwrap_or_default(),
            tpm: params.tpm.unwrap_or_default(),
            litellm_model,
            provider,
        })
    }

    // HTTPX's BasicAuth from URL userinfo overwrites the Authorization header, so it beats the api_key bearer
    #[must_use]
    pub fn credential(&self) -> Option<SecretString> {
        self.authorization.clone().or_else(|| {
            self.api_key
                .as_ref()
                .map(|key| SecretString::from(format!("Bearer {}", key.expose_secret())))
        })
    }

    // userinfo belongs to the URL it came with, so a call's own api_base replaces the Basic header too
    pub fn override_base(&mut self, api_base: &str) -> Result<(), DeploymentError> {
        let configured = configured_base(api_base, &self.model_group)?;
        self.api_base = String::from(configured.url.clone());
        self.endpoint = endpoint(configured.url);
        self.authorization = configured.authorization;
        Ok(())
    }

    pub fn endpoint_for(&self, resource: &str) -> Result<Url, DeploymentError> {
        let mut url = endpoint(configured_base(&self.api_base, &self.model_group)?.url);
        let path = format!(
            "{}/{}",
            url.path().trim_end_matches("/chat/completions"),
            resource
        );
        url.set_path(&path);
        Ok(url)
    }
}

fn provider_of(litellm_model: &str) -> Provider {
    litellm_model.strip_prefix("openai/").map_or_else(
        || Provider::Unsupported {
            model: litellm_model.to_owned(),
        },
        |model| Provider::OpenAi {
            model: model.to_owned(),
        },
    )
}

// the base as written without its query, or rebuilt from the parsed URL when userinfo had to come out
fn stored_api_base(configured: &ConfiguredBase, written: &str) -> String {
    if configured.authorization.is_none() {
        return written.split('?').next().unwrap_or_default().to_owned();
    }
    let mut stripped = configured.url.clone();
    stripped.set_query(None);
    String::from(stripped)
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

// the scheme and userinfo checks run here, once, so a bad api_base fails config load by key
fn configured_base(api_base: &str, model_name: &str) -> Result<ConfiguredBase, DeploymentError> {
    let url = Url::parse(api_base).map_err(|source| DeploymentError::ApiBase {
        model_name: model_name.to_owned(),
        source,
    })?;
    ConfiguredBase::parse(url, API_BASE_KEY).map_err(|source| DeploymentError::ConfiguredBase {
        model_name: model_name.to_owned(),
        source,
    })
}

fn endpoint(mut url: Url) -> Url {
    let path = format!("{}/chat/completions", url.path().trim_end_matches('/'));
    url.set_path(&path);
    url
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
