use secrecy::{ExposeSecret as _, SecretString};

use super::error::AgentSetupError;
use super::prepare::Preparation;
use super::resolve_credential;
use crate::agent_setup_profiles::Target;
use crate::claude_settings::choices::StaticToken;
use crate::config::error::ConfigError;
use crate::config::normalize_base_url;

pub const KEEP_CURRENT_KEY: &str = "Virtual key (press Enter to keep the current key)";
pub const NEW_KEY: &str = "Virtual key";

pub(crate) fn connect(
    request: &mut Preparation<'_>,
) -> Result<(String, StaticToken), AgentSetupError> {
    if !request.mode.edit_connection {
        return connection(request);
    }
    let base_url = prompt_url(request)?;
    let key = prompt_key(request, &base_url)?;
    let credential = resolve_credential(request.context, key.as_ref())?;
    Ok((base_url, credential))
}

fn offered_url(request: &Preparation<'_>) -> String {
    match request.saved.as_ref() {
        Some(saved) if !request.context.base_url_explicit => saved.base_url.clone(),
        _ => request.context.base_url.clone(),
    }
}

fn connection(request: &Preparation<'_>) -> Result<(String, StaticToken), AgentSetupError> {
    let record = request.saved.as_ref();
    let base_url = offered_url(request);
    let supplied = if request.context.api_key_from_token_file {
        None
    } else {
        request.context.api_key.as_ref()
    };
    let reusable = record
        .filter(|saved| saved.base_url == base_url)
        .map(|saved| &saved.api_key);
    let credential = match resolve_credential(request.context, supplied.or(reusable)) {
        Ok(credential) => credential,
        Err(error) => {
            if record.is_some_and(|saved| saved.base_url != base_url) && supplied.is_none() {
                return Err(AgentSetupError::GatewayChanged {
                    source: Box::new(error),
                });
            }
            return Err(error);
        }
    };
    Ok((base_url, credential))
}

fn prompt_url(request: &mut Preparation<'_>) -> Result<String, AgentSetupError> {
    let default_url = offered_url(request);
    let message = match request.target {
        Target::Claude => "Claude gateway URL",
        Target::Codex => "Codex gateway URL",
    };
    let entered = request.prompter.prompt_url(message, &default_url)?;
    let normalized = normalize_base_url(&entered).map_err(url_usage_error)?;
    Ok(normalized
        .strip_suffix("/v1")
        .unwrap_or(&normalized)
        .to_owned())
}

fn prompt_key(
    request: &mut Preparation<'_>,
    base_url: &str,
) -> Result<Option<SecretString>, AgentSetupError> {
    let supplied = if request.context.api_key_from_token_file {
        None
    } else {
        request.context.api_key.as_ref()
    };
    let kept = match supplied {
        Some(key) => Some(key.clone()),
        None => request
            .saved
            .as_ref()
            .filter(|saved| saved.base_url == base_url)
            .map(|saved| saved.api_key.clone()),
    };
    if let Some(key) = kept {
        let entered = request.prompter.prompt_secret(KEEP_CURRENT_KEY)?;
        if entered.expose_secret().is_empty() {
            return Ok(Some(key));
        }
        return Ok(Some(entered));
    }
    loop {
        let entered = request.prompter.prompt_secret(NEW_KEY)?;
        if !entered.expose_secret().is_empty() {
            return Ok(Some(entered));
        }
    }
}

fn url_usage_error(source: ConfigError) -> AgentSetupError {
    match source {
        ConfigError::Usage(text) => AgentSetupError::Usage(text),
        other @ (ConfigError::KeyNotSet(_)
        | ConfigError::NoHome
        | ConfigError::Runtime { .. }
        | ConfigError::Save(_)) => AgentSetupError::Config(Box::new(other)),
    }
}
