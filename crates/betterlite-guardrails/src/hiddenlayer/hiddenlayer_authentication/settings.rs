use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::HiddenlayerError;
use super::is_saas;
use super::token::TokenRequest;

pub const DEFAULT_API_BASE: &str = "https://api.hiddenlayer.ai";
pub const DEFAULT_AUTH_URL: &str = "https://auth.hiddenlayer.ai";
pub const DEFAULT_AUTH_TIMEOUT: Duration = Duration::from_secs(30);

const API_BASE_ENV: &str = "HIDDENLAYER_API_BASE";
const AUTH_URL_ENV: &str = "HIDDENLAYER_AUTH_URL";
const CLIENT_ID_ENV: &str = "HIDDENLAYER_CLIENT_ID";
const CLIENT_SECRET_ENV: &str = "HIDDENLAYER_CLIENT_SECRET";

#[derive(Debug, Clone, Default)]
pub struct HiddenlayerCredentials {
    pub api_id: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub auth_url: Option<String>,
    pub timeout: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct HiddenlayerSettings {
    pub(crate) api_base: String,
    pub(crate) auth_url: String,
    pub(crate) api_id: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) auth_timeout: Duration,
}

pub fn resolve<L>(
    credentials: &HiddenlayerCredentials,
    lookup: &L,
) -> Result<HiddenlayerSettings, HiddenlayerError>
where
    L: EnvironmentLookup,
{
    let settings = HiddenlayerSettings {
        api_base: resolve_text(credentials.api_base.as_deref(), API_BASE_ENV, lookup)
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        auth_url: resolve_text(credentials.auth_url.as_deref(), AUTH_URL_ENV, lookup)
            .unwrap_or_else(|| DEFAULT_AUTH_URL.to_owned()),
        api_id: resolve_text(credentials.api_id.as_deref(), CLIENT_ID_ENV, lookup),
        api_key: credentials
            .api_key
            .clone()
            .filter(|key| !key.expose_secret().is_empty())
            .or_else(|| {
                lookup
                    .lookup(CLIENT_SECRET_ENV)
                    .filter(|key| !key.is_empty())
                    .map(SecretString::from)
            }),
        auth_timeout: resolve_timeout(credentials.timeout)?,
    };
    if is_saas(&settings.api_base) {
        settings.require_saas_credentials()?;
    }
    Ok(settings)
}

fn resolve_text<L>(configured: Option<&str>, env: &str, lookup: &L) -> Option<String>
where
    L: EnvironmentLookup,
{
    configured
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| lookup.lookup(env).filter(|value| !value.is_empty()))
}

fn resolve_timeout(timeout: Option<f64>) -> Result<Duration, HiddenlayerError> {
    timeout.map_or_else(
        || Ok(DEFAULT_AUTH_TIMEOUT),
        |seconds| {
            Duration::try_from_secs_f64(seconds)
                .map_err(|source| HiddenlayerError::InvalidTimeout { source })
        },
    )
}

impl HiddenlayerSettings {
    fn require_saas_credentials(&self) -> Result<(), HiddenlayerError> {
        if self.api_id.as_deref().is_none_or(str::is_empty) {
            return Err(HiddenlayerError::MissingApiId);
        }
        if self
            .api_key
            .as_ref()
            .is_none_or(|key| key.expose_secret().is_empty())
        {
            return Err(HiddenlayerError::MissingApiKey);
        }
        Ok(())
    }

    pub(crate) fn token_request(&self) -> Result<TokenRequest, HiddenlayerError> {
        let Some(api_id) = self.api_id.clone() else {
            return Err(HiddenlayerError::MissingApiId);
        };
        let Some(api_key) = self.api_key.clone() else {
            return Err(HiddenlayerError::MissingApiKey);
        };
        Ok(TokenRequest {
            url: format!(
                "{}/oauth2/token?grant_type=client_credentials",
                self.auth_url
            ),
            api_id,
            api_key,
            timeout: self.auth_timeout,
        })
    }
}
