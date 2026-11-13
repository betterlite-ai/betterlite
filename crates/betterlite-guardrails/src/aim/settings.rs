use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use super::error::AimError;
use super::stream::AimSocketConnector;
use super::transport::AimPoster;

pub const DEFAULT_API_BASE: &str = "https://api.aim.security";
pub const DEFAULT_LITELLM_VERSION: &str = "1.105.1";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 3] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AimHook {
    PreCall,
    Moderation,
    Output,
}

impl AimHook {
    #[must_use]
    pub const fn header_value(self) -> &'static str {
        match self {
            Self::PreCall => "pre_call",
            Self::Moderation => "moderation",
            Self::Output => "output",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AimParams {
    pub guardrail_name: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub inspect_embeddings: Option<bool>,
    pub litellm_version: Option<String>,
}

pub struct AimGuardrail {
    pub guardrail_name: Option<String>,
    pub api_key: SecretString,
    pub api_base: String,
    pub ws_api_base: String,
    pub inspect_embeddings: bool,
    pub litellm_version: String,
    pub timeout: Option<Duration>,
    pub poster: Arc<dyn AimPoster>,
    pub connector: Arc<dyn AimSocketConnector>,
}

fn header_value(value: &str) -> Result<HeaderValue, AimError> {
    HeaderValue::from_str(value).map_err(AimError::InvalidHeader)
}

impl AimGuardrail {
    pub fn new<L>(
        params: AimParams,
        environment: &L,
        poster: Arc<dyn AimPoster>,
        connector: Arc<dyn AimSocketConnector>,
        timeout: Option<Duration>,
    ) -> Result<Self, AimError>
    where
        L: EnvironmentLookup,
    {
        let api_key = match params.api_key.filter(|key| !key.expose_secret().is_empty()) {
            Some(key) => key,
            None => environment
                .lookup("AIM_API_KEY")
                .filter(|key| !key.is_empty())
                .map(SecretString::from)
                .ok_or(AimError::MissingSecrets)?,
        };
        let api_base = params
            .api_base
            .filter(|base| !base.is_empty())
            .or_else(|| {
                environment
                    .lookup("AIM_API_BASE")
                    .filter(|base| !base.is_empty())
            })
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        let ws_api_base = api_base
            .replace("http://", "ws://")
            .replace("https://", "wss://");
        Ok(Self {
            guardrail_name: params.guardrail_name,
            api_key,
            api_base,
            ws_api_base,
            inspect_embeddings: params.inspect_embeddings.unwrap_or(false),
            litellm_version: params
                .litellm_version
                .unwrap_or_else(|| DEFAULT_LITELLM_VERSION.to_owned()),
            timeout,
            poster,
            connector,
        })
    }

    pub fn build_headers(
        &self,
        hook: AimHook,
        key_alias: Option<&str>,
        user_email: Option<&str>,
        call_id: Option<&str>,
    ) -> Result<HeaderMap, AimError> {
        let mut headers = HeaderMap::new();
        let mut authorization = header_value(&format!("Bearer {}", self.api_key.expose_secret()))?;
        authorization.set_sensitive(true);
        headers.insert(HeaderName::from_static("authorization"), authorization);
        headers.insert(
            HeaderName::from_static("x-aim-litellm-hook"),
            header_value(hook.header_value())?,
        );
        headers.insert(
            HeaderName::from_static("x-aim-litellm-version"),
            header_value(&self.litellm_version)?,
        );
        for (name, value) in [
            ("x-aim-call-id", call_id),
            ("x-aim-user-email", user_email),
            ("x-aim-gateway-key-alias", key_alias),
        ] {
            if let Some(value) = value.filter(|value| !value.is_empty()) {
                headers.insert(HeaderName::from_static(name), header_value(value)?);
            }
        }
        Ok(headers)
    }
}
