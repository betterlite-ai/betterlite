use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_prompts::message_content::Message;
use betterlite_prompts::message_fields::last_user::get_last_user_message;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

pub mod error;
pub mod wire;

use error::{JavelinError, JavelinInitializationError};
use wire::{
    JavelinExtraValue, JavelinGuardInput, JavelinGuardRequest, JavelinGuardResponse, find_rejection,
};

pub const DEFAULT_API_BASE: &str = "https://api-dev.javelin.live";
pub const DEFAULT_API_VERSION: &str = "v1";
pub const DEFAULT_GUARDRAIL_NAME: &str = "trustsafety";
// LiteLLM's AsyncHTTPHandler falls back to httpx's 600s read/write timeout when no guardrail timeout is configured; see docs/deviations/B3777.yaml for the connect-timeout sub-component this doesn't model.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
// LiteLLM strips this logging-only key out of metadata before forwarding it to Javelin.
const LOGGING_METADATA_KEY: &str = "standard_logging_guardrail_information";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JavelinEventHook {
    PreCall,
}

pub const SUPPORTED_EVENT_HOOKS: [JavelinEventHook; 1] = [JavelinEventHook::PreCall];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JavelinConfigModel {
    pub guard_name: Option<String>,
    pub api_version: Option<String>,
    pub metadata: Option<BTreeMap<String, JavelinExtraValue>>,
    pub application: Option<String>,
    pub config: Option<BTreeMap<String, JavelinExtraValue>>,
}

impl Default for JavelinConfigModel {
    fn default() -> Self {
        Self {
            guard_name: None,
            api_version: Some(DEFAULT_API_VERSION.to_owned()),
            metadata: None,
            application: None,
            config: None,
        }
    }
}

impl GuardrailConfigModelIdentity for JavelinConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Javelin Guardrails"
    }
}

pub trait CallbackRegistry {
    fn add_litellm_callback(&mut self, guardrail: Arc<JavelinGuardrail>);
}

pub trait AppliedGuardrailsHeader {
    fn record_applied_guardrail(&mut self, guardrail_name: &str);
}

#[derive(Debug, Default)]
pub struct JavelinGuardrailParams {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub default_on: Option<bool>,
    pub guardrail_name: Option<String>,
    pub javelin_guard_name: Option<String>,
    pub metadata: Option<BTreeMap<String, JavelinExtraValue>>,
    pub config: Option<BTreeMap<String, JavelinExtraValue>>,
    pub application: Option<String>,
}

#[derive(Debug)]
pub struct JavelinGuardrail {
    pub api_key: SecretString,
    pub api_base: String,
    pub api_version: String,
    pub default_on: bool,
    pub guardrail_name: String,
    pub javelin_guard_name: String,
    pub metadata: Option<BTreeMap<String, JavelinExtraValue>>,
    pub config: Option<BTreeMap<String, JavelinExtraValue>>,
    pub application: Option<String>,
    pub client: reqwest::Client,
    pub timeout: Duration,
}

impl JavelinGuardrail {
    #[must_use]
    pub fn new(
        params: JavelinGuardrailParams,
        get_secret_str: fn(&str) -> String,
        client: reqwest::Client,
        timeout: Duration,
    ) -> Self {
        let api_key = params
            .api_key
            .unwrap_or_else(|| SecretString::from(get_secret_str("JAVELIN_API_KEY")));
        let api_base = params.api_base.unwrap_or_else(|| {
            let from_env = get_secret_str("JAVELIN_API_BASE");
            if from_env.is_empty() {
                DEFAULT_API_BASE.to_owned()
            } else {
                from_env
            }
        });
        let guardrail_name = params
            .guardrail_name
            .unwrap_or_else(|| DEFAULT_GUARDRAIL_NAME.to_owned());
        let javelin_guard_name = params
            .javelin_guard_name
            .unwrap_or_else(|| guardrail_name.clone());
        Self {
            api_key,
            api_base,
            api_version: params
                .api_version
                .unwrap_or_else(|| DEFAULT_API_VERSION.to_owned()),
            default_on: params.default_on.unwrap_or(true),
            guardrail_name,
            javelin_guard_name,
            metadata: params.metadata,
            config: params.config,
            application: params.application,
            client,
            timeout,
        }
    }

    fn clean_metadata(&self) -> BTreeMap<String, JavelinExtraValue> {
        self.metadata
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|(key, _)| key != LOGGING_METADATA_KEY)
            .collect()
    }

    pub fn prepare_headers(&self) -> Result<HeaderMap, JavelinError> {
        let mut headers = HeaderMap::new();
        let mut api_key = HeaderValue::from_str(self.api_key.expose_secret())
            .map_err(|source| JavelinError::InvalidApiKeyHeader { source })?;
        api_key.set_sensitive(true);
        headers.insert(HeaderName::from_static("x-javelin-apikey"), api_key);
        if let Some(application) = self
            .application
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            let value = HeaderValue::from_str(application)
                .map_err(|source| JavelinError::InvalidApplicationHeader { source })?;
            headers.insert(HeaderName::from_static("x-javelin-application"), value);
        }
        Ok(headers)
    }

    pub async fn call_javelin_guard(&self, request: &JavelinGuardRequest) -> JavelinGuardResponse {
        match self.try_call_javelin_guard(request).await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(%error, "Javelin guard API call failed; failing open");
                JavelinGuardResponse::default()
            }
        }
    }

    async fn try_call_javelin_guard(
        &self,
        request: &JavelinGuardRequest,
    ) -> Result<JavelinGuardResponse, JavelinError> {
        let headers = self.prepare_headers()?;
        let url = format!(
            "{}/{}/guardrail/{}/apply",
            self.api_base.trim_end_matches('/'),
            self.api_version,
            self.javelin_guard_name
        );
        let mut response = self
            .client
            .post(url)
            .headers(headers)
            .json(request)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(JavelinError::HttpStatus { status });
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(JavelinError::ResponseTooLarge {
                    limit: MAX_RESPONSE_BYTES,
                });
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|source| JavelinError::ResponseJson { source })
    }

    pub async fn async_pre_call_hook<A>(
        &self,
        messages: Option<&[Message]>,
        applied_guardrails: &mut A,
    ) -> Result<(), JavelinError>
    where
        A: AppliedGuardrailsHeader,
    {
        let Some(text) = messages.and_then(get_last_user_message) else {
            return Ok(());
        };
        let request = JavelinGuardRequest {
            input: JavelinGuardInput { text },
            config: self.config.clone().unwrap_or_default(),
            metadata: self.clean_metadata(),
        };
        let response = self.call_javelin_guard(&request).await;
        if let Some(assessment) = find_rejection(&response) {
            let reject_prompt = assessment
                .results
                .reject_prompt
                .clone()
                .filter(|prompt| !prompt.is_empty())
                .unwrap_or_else(|| {
                    format!(
                        "Request blocked by Javelin guardrails due to {} violation.",
                        self.guardrail_name
                    )
                });
            return Err(JavelinError::Rejected {
                reject_prompt,
                javelin_guardrail_response: response,
            });
        }
        applied_guardrails.record_applied_guardrail(&self.guardrail_name);
        Ok(())
    }
}

fn request_error(source: reqwest::Error) -> JavelinError {
    if source.is_timeout() {
        JavelinError::Timeout { source }
    } else {
        JavelinError::Request { source }
    }
}

pub fn initialize_guardrail<R>(
    params: JavelinGuardrailParams,
    get_secret_str: fn(&str) -> String,
    client: reqwest::Client,
    timeout: Duration,
    registry: &mut R,
) -> Result<Arc<JavelinGuardrail>, JavelinInitializationError>
where
    R: CallbackRegistry,
{
    if params.javelin_guard_name.is_none() {
        return Err(JavelinInitializationError::MissingGuardName);
    }
    // LiteLLM's initialize_guardrail resolves default_on to False when unset, unlike the constructor's own True default.
    let default_on = Some(params.default_on.unwrap_or(false));
    let resolved_params = JavelinGuardrailParams {
        default_on,
        ..params
    };
    let guardrail = Arc::new(JavelinGuardrail::new(
        resolved_params,
        get_secret_str,
        client,
        timeout,
    ));
    registry.add_litellm_callback(Arc::clone(&guardrail));
    Ok(guardrail)
}

#[path = "javelin/tests.rs"]
mod tests;
