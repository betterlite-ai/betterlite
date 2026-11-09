use std::num::ParseFloatError;

use betterlite_config::sections::langfuse_settings::Settings;
use secrecy::SecretString;

use crate::logging_privacy::CallbackMetadataError;
use crate::logging_privacy::callbacks::validate_environment;

const CLOUD_HOST: &str = "https://cloud.langfuse.com";

#[derive(Debug, thiserror::Error)]
pub enum LangfuseSettingsError {
    #[error("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS='{raw}' must be a whole number of seconds")]
    UnusablePromptCacheTtl { raw: String },
    #[error("LANGFUSE_TIMEOUT='{raw}' is not a number of seconds")]
    InvalidTimeout {
        raw: String,
        #[source]
        source: ParseFloatError,
    },
    #[error(transparent)]
    Environment {
        #[from]
        source: CallbackMetadataError,
    },
    #[error(transparent)]
    Flag {
        #[from]
        source: betterlite_env::EnvError,
    },
}

// The caller's own values, before the environment fills the gaps LiteLLM lets it fill.
#[derive(Clone, Copy)]
pub struct CredentialRequest<'request> {
    pub public_key: Option<&'request str>,
    pub secret: Option<&'request str>,
    pub secret_key: Option<&'request str>,
    pub host: Option<&'request str>,
    pub allow_env_credentials: bool,
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub public_key: Option<String>,
    pub secret_key: Option<SecretString>,
    pub host: String,
}

#[must_use]
pub fn resolve_credentials(settings: &Settings, request: CredentialRequest<'_>) -> Credentials {
    let argument_secret = [request.secret, request.secret_key]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .map(SecretString::from);
    let (public_key, secret_key) = if !request.allow_env_credentials && request.host.is_some() {
        (request.public_key.map(str::to_owned), argument_secret)
    } else {
        let resolved_public = request
            .public_key
            .map(str::to_owned)
            .filter(|value| !value.is_empty())
            .or_else(|| settings.public_key.clone());
        (
            resolved_public,
            argument_secret.or_else(|| settings.secret_key.clone()),
        )
    };
    Credentials {
        public_key,
        secret_key,
        host: resolve_host(settings, request.host),
    }
}

#[must_use]
pub fn resolve_host(settings: &Settings, host: Option<&str>) -> String {
    let resolved = [host, settings.host.as_deref(), settings.base_url.as_deref()]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .unwrap_or(CLOUD_HOST);
    if resolved.starts_with("http://") || resolved.starts_with("https://") {
        resolved.to_owned()
    } else {
        format!("http://{resolved}")
    }
}

pub fn resolve_environment(
    settings: &Settings,
    explicit: Option<&str>,
) -> Result<Option<String>, LangfuseSettingsError> {
    let trimmed = explicit.map(str::trim).filter(|value| !value.is_empty());
    if let Some(value) = trimmed {
        validate_environment(value)?;
        return Ok(Some(value.to_owned()));
    }
    let Some(raw) = settings
        .tracing_environment
        .as_deref()
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let value = raw.trim();
    match validate_environment(value) {
        Ok(()) => Ok(Some(value.to_owned())),
        Err(source) => {
            tracing::warn!(
                "Ignoring invalid LANGFUSE_TRACING_ENVIRONMENT='{raw}' for the langfuse callback: {source}. Traces will be sent to Langfuse's default environment."
            );
            Ok(Some("default".to_owned()))
        }
    }
}

pub fn debug(settings: &Settings) -> Result<bool, LangfuseSettingsError> {
    let Some(raw) = settings.debug.as_deref() else {
        return Ok(false);
    };
    Ok(betterlite_env::flag("LANGFUSE_DEBUG", raw)?.unwrap_or(false))
}

pub fn mock_mode(settings: &Settings) -> Result<bool, LangfuseSettingsError> {
    let Some(raw) = settings.mock.as_deref() else {
        return Ok(false);
    };
    let enabled = betterlite_env::flag("LANGFUSE_MOCK", raw)?.unwrap_or(false);
    if enabled {
        tracing::info!("LANGFUSE Mock Mode: ENABLED - API calls will be mocked");
    }
    Ok(enabled)
}

pub fn warn_if_upstream_configured(settings: &Settings) {
    if settings.upstream_secret_key.is_some() {
        tracing::warn!(
            "UPSTREAM_LANGFUSE_* is no longer supported: the langfuse callback moved to SDK v4, which has no second ingestion client. The values are ignored."
        );
    }
}

#[must_use]
pub fn release(settings: &Settings) -> Option<String> {
    settings
        .release
        .clone()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let commits = &settings.commits;
            [
                &commits.render_git_commit,
                &commits.ci_commit_sha,
                &commits.circle_sha1,
                &commits.source_version,
                &commits.travis_commit,
                &commits.git_commit,
                &commits.github_sha,
                &commits.bitbucket_commit,
                &commits.build_sourceversion,
                &commits.drone_commit_sha,
            ]
            .into_iter()
            .find_map(Option::as_ref)
            .cloned()
        })
}

pub mod export;

mod tests;
