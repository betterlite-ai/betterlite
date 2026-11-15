use std::time::{Duration, TryFromFloatSecsError};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_http::user_url::error::UrlParseError;
use betterlite_http::user_url::split::urlparse;
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

use super::DEFAULT_API_BASE;
use super::DEFAULT_TIMEOUT;
use super::init::SingulrArgs;

pub const API_BASE_ENV: &str = "SINGULR_API_BASE";
pub const API_KEY_ENV: &str = "SINGULR_API_KEY";
pub const APPLICATION_ID_ENV: &str = "SINGULR_ENFORCEMENT_ENTITY_ID";
pub const GUARDRAIL_ID_ENV: &str = "SINGULR_GUARDRAIL_ID";
pub const BLOCK_ON_ERROR_ENV: &str = "SINGULR_BLOCK_ON_ERROR";

const LOCAL_HOSTS: [&str; 2] = ["localhost", "127.0.0.1"];

#[derive(Debug, Clone)]
pub struct SingulrSettings {
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub application_id: Option<String>,
    pub guardrail_id: Option<String>,
    pub block_on_error: bool,
    pub timeout: Duration,
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("Singulr: api_base {api_base} is not a valid URL")]
    InvalidBase {
        api_base: String,
        #[source]
        source: UrlParseError,
    },
    #[error(
        "Singulr: api_base {api_base} uses plain HTTP for a non-local endpoint; the guardrail token and the conversation would travel in clear text, so this endpoint must use HTTPS"
    )]
    PlainHttp { api_base: String },
    #[error("the Singulr timeout is not a valid duration")]
    Timeout {
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("environment variable {name} must be a boolean")]
    BlockOnError {
        name: &'static str,
        #[source]
        source: betterlite_env::EnvError,
    },
}

impl SingulrSettings {
    pub fn resolve<L>(args: &SingulrArgs, environment: &L) -> Result<Self, SettingsError>
    where
        L: EnvironmentLookup,
    {
        let api_base = resolve_text(args.singulr_api_base.as_deref(), environment, API_BASE_ENV)
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        let trimmed = api_base.trim().trim_end_matches('/').to_owned();
        // urlparse raises ValueError on a netloc it rejects, and LiteLLM's init fails there before the plain-HTTP check
        let parsed = urlparse(&trimmed, "").map_err(|source| SettingsError::InvalidBase {
            api_base: trimmed.clone(),
            source,
        })?;
        let local = parsed
            .hostname()
            .is_some_and(|host| LOCAL_HOSTS.contains(&host.as_str()));
        if parsed.scheme == "http" && !local {
            return Err(SettingsError::PlainHttp { api_base: trimmed });
        }
        let api_key = resolve_text(
            args.singulr_api_key
                .as_ref()
                .map(ExposeSecret::expose_secret),
            environment,
            API_KEY_ENV,
        )
        .map(SecretString::from);
        let timeout = resolve_timeout(args.timeout)?;
        Ok(Self {
            api_base: trimmed,
            api_key,
            application_id: resolve_text(
                args.singulr_application_id.as_deref(),
                environment,
                APPLICATION_ID_ENV,
            ),
            guardrail_id: resolve_text(
                args.singulr_guardrail_id.as_deref(),
                environment,
                GUARDRAIL_ID_ENV,
            ),
            block_on_error: resolve_block_on_error(args.block_on_error, environment)?,
            timeout,
        })
    }
}

fn resolve_timeout(configured: Option<f64>) -> Result<Duration, SettingsError> {
    match configured {
        Some(seconds) => {
            Duration::try_from_secs_f64(seconds).map_err(|source| SettingsError::Timeout { source })
        }
        None => Ok(DEFAULT_TIMEOUT),
    }
}

// an explicit value is used as given; only an absent one reads the environment, where an empty value counts as unset
fn resolve_text<L>(explicit: Option<&str>, environment: &L, name: &str) -> Option<String>
where
    L: EnvironmentLookup,
{
    explicit
        .map(str::to_owned)
        .or_else(|| environment.lookup(name).filter(|value| !value.is_empty()))
}

fn resolve_block_on_error<L>(explicit: Option<bool>, environment: &L) -> Result<bool, SettingsError>
where
    L: EnvironmentLookup,
{
    if let Some(value) = explicit {
        return Ok(value);
    }
    let Some(text) = environment.lookup(BLOCK_ON_ERROR_ENV) else {
        return Ok(true);
    };
    let parsed = betterlite_env::flag(BLOCK_ON_ERROR_ENV, &text).map_err(|source| {
        SettingsError::BlockOnError {
            name: BLOCK_ON_ERROR_ENV,
            source,
        }
    })?;
    Ok(parsed.unwrap_or(true))
}
