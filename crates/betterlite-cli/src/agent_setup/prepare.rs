use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;
use soft_canonicalize::soft_canonicalize;

use super::error::AgentSetupError;
use super::listing::{ListingRequest, connection_listing, starting_model, validated_model};
use super::pickers::{pick_claude_model, pick_codex_model};
use super::prompt::Prompter;
use super::resolve_credential;
use crate::agent_setup_profiles::error::AgentSetupProfilesError;
use crate::agent_setup_profiles::{SavedSetup, Target, settings_path_for};
use crate::claude_settings::choices::StaticToken;
use crate::cli_context::CliContext;
use crate::config::error::ConfigError;
use crate::config::normalize_base_url;
use crate::pi::ListedModel;

pub const CODEX_MODEL_REQUIRED: &str =
    "Missing option '--model'. First-time Codex setup needs a starting model";
pub const KEEP_CURRENT_KEY: &str = "Virtual key (press Enter to keep the current key)";
pub const NEW_KEY: &str = "Virtual key";

#[derive(Clone, Copy)]
pub struct SetupMode {
    pub interactive: bool,
    pub edit_connection: bool,
}

pub struct Preparation<'request> {
    pub target: Target,
    pub saved: Option<SavedSetup>,
    pub model: Option<String>,
    pub default_model: bool,
    pub mode: SetupMode,
    pub context: &'request CliContext,
    pub environment: &'request (dyn EnvironmentLookup + Send + Sync),
    pub prompter: &'request mut dyn Prompter,
    pub stdin_is_terminal: bool,
}

pub struct PreparedSetup {
    pub saved: SavedSetup,
    pub listing: Vec<ListedModel>,
}

pub async fn prepare(request: &mut Preparation<'_>) -> Result<PreparedSetup, AgentSetupError> {
    let default = default_model(request);
    if request.target == Target::Codex && default.is_none() && !request.mode.interactive {
        return Err(AgentSetupError::Usage(CODEX_MODEL_REQUIRED.to_owned()));
    }
    let (base_url, credential) = connect(request)?;
    listed_setup(request, base_url, credential, default).await
}

fn default_model(request: &Preparation<'_>) -> Option<String> {
    if request.default_model {
        return None;
    }
    if request.model.is_some() {
        return request.model.clone();
    }
    request.saved.as_ref()?.model.clone()
}

fn connect(request: &mut Preparation<'_>) -> Result<(String, StaticToken), AgentSetupError> {
    if !request.mode.edit_connection {
        return connection(request);
    }
    let base_url = prompt_url(request)?;
    let key = prompt_key(request, &base_url)?;
    let credential = resolve_credential(request.context, key.as_ref())?;
    Ok((base_url, credential))
}

async fn listed_setup(
    request: &mut Preparation<'_>,
    base_url: String,
    credential: StaticToken,
    default: Option<String>,
) -> Result<PreparedSetup, AgentSetupError> {
    let repair = request.saved.is_some() && !request.mode.interactive && request.stdin_is_terminal;
    let mut listing_request = ListingRequest {
        context: request.context,
        base_url: &base_url,
        credential,
        target: request.target,
        repair,
        environment: request.environment,
        prompter: &mut *request.prompter,
    };
    let (active, listing) = connection_listing(&mut listing_request).await?;
    let repair_model = repair
        && default
            .as_deref()
            .is_some_and(|model| starting_model(model, &listing).is_none());
    let chosen = choose_model(request, &listing, default, repair_model)?;
    let validated = validated_model(chosen.as_deref(), &listing, &base_url)?;
    let model = match request.target {
        Target::Claude => source_name(&listing, validated),
        Target::Codex => Some(chosen.ok_or_else(|| {
            AgentSetupError::Usage(
                "First-time Codex setup needs --model. Run `lite configure` for the model picker"
                    .to_owned(),
            )
        })?),
    };
    let settings_path = resolved_settings_path(request.target, request.environment)?;
    let saved = SavedSetup::new(request.target, settings_path, base_url, active.token, model)
        .map_err(|source| AgentSetupError::InvalidSetup {
            source: Box::new(source),
        })?;
    Ok(PreparedSetup { saved, listing })
}

fn source_name(listing: &[ListedModel], validated: Option<&str>) -> Option<String> {
    let id = validated?;
    listing.iter().find(|listed| listed.id == id).map(|listed| {
        listed
            .source_model
            .clone()
            .unwrap_or_else(|| listed.id.clone())
    })
}

fn choose_model(
    request: &mut Preparation<'_>,
    listing: &[ListedModel],
    default: Option<String>,
    repair_model: bool,
) -> Result<Option<String>, AgentSetupError> {
    if !(request.mode.interactive || repair_model) {
        return Ok(default);
    }
    if request.target == Target::Claude {
        let names: Vec<String> = listing
            .iter()
            .map(|listed| {
                listed
                    .source_model
                    .clone()
                    .unwrap_or_else(|| listed.id.clone())
            })
            .collect();
        return pick_claude_model(request.prompter, &names, default.as_deref());
    }
    let ids: Vec<String> = listing.iter().map(|listed| listed.id.clone()).collect();
    Ok(Some(pick_codex_model(
        request.prompter,
        &ids,
        default.as_deref(),
    )?))
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
        if entered.is_empty() {
            return Ok(Some(key));
        }
        return Ok(Some(SecretString::from(entered)));
    }
    loop {
        let entered = request.prompter.prompt_secret(NEW_KEY)?;
        if !entered.is_empty() {
            return Ok(Some(SecretString::from(entered)));
        }
    }
}

fn url_usage_error(source: ConfigError) -> AgentSetupError {
    match source {
        ConfigError::Usage(text) => AgentSetupError::Usage(text),
        other @ (ConfigError::KeyNotSet(_)
        | ConfigError::NoHome
        | ConfigError::Runtime { .. }
        | ConfigError::Save(_)) => AgentSetupError::Usage(other.to_string()),
    }
}

fn resolved_settings_path(
    target: Target,
    environment: &dyn EnvironmentLookup,
) -> Result<String, AgentSetupError> {
    let path = settings_path_for(target, environment).ok_or(AgentSetupError::Profile(Box::new(
        AgentSetupProfilesError::Path(target),
    )))?;
    let resolved = soft_canonicalize(&path).map_err(|source| {
        AgentSetupError::Profile(Box::new(AgentSetupProfilesError::ResolveSettings(
            target,
            path.clone(),
            source,
        )))
    })?;
    resolved
        .to_str()
        .map(str::to_owned)
        .ok_or(AgentSetupError::SettingsPathNotUtf8 { target })
}
