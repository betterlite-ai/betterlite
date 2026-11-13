use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_prompts::arguments::json::Object;
use thiserror::Error;

use crate::mcp_token_claims::ClaimSettings;

mod tests;

pub const DEFAULT_ISSUER: &str = "litellm";
pub const DEFAULT_AUDIENCE: &str = "mcp";
pub const DEFAULT_TTL_SECONDS: i64 = 300;
// LiteLLM passes timeout=None to httpx, whose default is Timeout(600, connect=5); the connect limit belongs to the injected client.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
const ISSUER_ENV: &str = "MCP_JWT_ISSUER";
const AUDIENCE_ENV: &str = "MCP_JWT_AUDIENCE";
const TTL_ENV: &str = "MCP_JWT_TTL_SECONDS";
const EXTERNAL_URL_ENV: &str = "LITELLM_EXTERNAL_URL";

#[derive(Debug, Clone, Default)]
pub struct SignerOptions {
    pub issuer: Option<String>,
    pub audience: Option<String>,
    pub ttl_seconds: Option<i64>,
    pub access_token_discovery_uri: Option<String>,
    pub token_introspection_endpoint: Option<String>,
    pub verify_issuer: Option<String>,
    pub verify_audience: Option<String>,
    pub end_user_claim_sources: Option<Vec<String>>,
    pub add_claims: Option<Object>,
    pub set_claims: Option<Object>,
    pub remove_claims: Option<Vec<String>>,
    pub channel_token_audience: Option<String>,
    pub channel_token_ttl: Option<i64>,
    pub required_claims: Option<Vec<String>>,
    pub optional_claims: Option<Vec<String>>,
    pub debug_headers: bool,
    pub allowed_scopes: Option<Vec<String>>,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct SignerSettings {
    pub issuer: String,
    pub audience: String,
    pub ttl_seconds: i64,
    pub access_token_discovery_uri: Option<String>,
    pub token_introspection_endpoint: Option<String>,
    pub verify_issuer: Option<String>,
    pub verify_audience: Option<String>,
    pub end_user_claim_sources: Vec<String>,
    pub add_claims: Object,
    pub set_claims: Object,
    pub remove_claims: Vec<String>,
    pub channel_token_audience: Option<String>,
    pub channel_token_ttl: i64,
    pub required_claims: Vec<String>,
    pub optional_claims: Vec<String>,
    pub debug_headers: bool,
    pub allowed_scopes: Option<Vec<String>>,
    pub timeout: Duration,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SettingsError {
    #[error("ttl_seconds must be > 0, got {ttl_seconds}")]
    Ttl { ttl_seconds: i64 },
    #[error(transparent)]
    Environment(#[from] betterlite_env::EnvError),
}

impl SignerSettings {
    pub fn resolve(
        options: &SignerOptions,
        env: &dyn EnvironmentLookup,
    ) -> Result<Self, SettingsError> {
        let ttl_seconds = resolve_ttl(options, env)?;
        Ok(Self {
            issuer: first_set(
                options.issuer.as_deref(),
                env,
                &[ISSUER_ENV, EXTERNAL_URL_ENV],
                DEFAULT_ISSUER,
            ),
            audience: first_set(
                options.audience.as_deref(),
                env,
                &[AUDIENCE_ENV],
                DEFAULT_AUDIENCE,
            ),
            ttl_seconds,
            access_token_discovery_uri: options.access_token_discovery_uri.clone(),
            token_introspection_endpoint: options.token_introspection_endpoint.clone(),
            verify_issuer: options.verify_issuer.clone(),
            verify_audience: options.verify_audience.clone(),
            end_user_claim_sources: options
                .end_user_claim_sources
                .clone()
                .filter(|sources| !sources.is_empty())
                .unwrap_or_else(|| vec!["token:sub".to_owned(), "litellm:user_id".to_owned()]),
            add_claims: options.add_claims.clone().unwrap_or_default(),
            set_claims: options.set_claims.clone().unwrap_or_default(),
            remove_claims: options.remove_claims.clone().unwrap_or_default(),
            channel_token_audience: options.channel_token_audience.clone(),
            channel_token_ttl: options.channel_token_ttl.unwrap_or(ttl_seconds),
            required_claims: options.required_claims.clone().unwrap_or_default(),
            optional_claims: options.optional_claims.clone().unwrap_or_default(),
            debug_headers: options.debug_headers,
            allowed_scopes: options.allowed_scopes.clone(),
            timeout: options.timeout.unwrap_or(DEFAULT_TIMEOUT),
        })
    }

    #[must_use]
    pub fn claim_settings(&self) -> ClaimSettings {
        ClaimSettings {
            issuer: self.issuer.clone(),
            audience: self.audience.clone(),
            ttl_seconds: self.ttl_seconds,
            allowed_scopes: self.allowed_scopes.clone(),
            add_claims: self.add_claims.clone(),
            set_claims: self.set_claims.clone(),
            remove_claims: self.remove_claims.clone(),
            required_claims: self.required_claims.clone(),
            optional_claims: self.optional_claims.clone(),
            end_user_claim_sources: self.end_user_claim_sources.clone(),
        }
    }
}

fn first_set(
    option: Option<&str>,
    env: &dyn EnvironmentLookup,
    names: &[&str],
    default: &str,
) -> String {
    let from_option = option.filter(|value| !value.is_empty());
    let from_env = names
        .iter()
        .find_map(|name| env.lookup(name).filter(|value| !value.is_empty()));
    from_option
        .map(str::to_owned)
        .or(from_env)
        .unwrap_or_else(|| default.to_owned())
}

fn resolve_ttl(options: &SignerOptions, env: &dyn EnvironmentLookup) -> Result<i64, SettingsError> {
    if let Some(ttl_seconds) = options.ttl_seconds {
        return positive_ttl(ttl_seconds);
    }
    let Some(text) = env.lookup(TTL_ENV) else {
        return Ok(DEFAULT_TTL_SECONDS);
    };
    let ttl_seconds = betterlite_env::number::<i64>(TTL_ENV, &text)?.unwrap_or(DEFAULT_TTL_SECONDS);
    positive_ttl(ttl_seconds)
}

const fn positive_ttl(ttl_seconds: i64) -> Result<i64, SettingsError> {
    if ttl_seconds <= 0 {
        return Err(SettingsError::Ttl { ttl_seconds });
    }
    Ok(ttl_seconds)
}
