use betterlite_config::loader::EnvironmentLookup;
use soft_canonicalize::soft_canonicalize;

use super::connection::connect;
use super::error::AgentSetupError;
use super::listing::{ListingRequest, connection_listing, starting_model, validated_model};
use super::pickers::{pick_claude_model, pick_codex_model};
use super::prompt::Prompter;
use crate::agent_setup_profiles::error::AgentSetupProfilesError;
use crate::agent_setup_profiles::{SavedSetup, Target, settings_path_for};
use crate::claude_settings::choices::StaticToken;
use crate::cli_context::CliContext;
use crate::pi::ListedModel;

pub const CODEX_MODEL_REQUIRED: &str =
    "Missing option '--model'. First-time Codex setup needs a starting model";

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

enum CodexStart {
    Ready(String),
    Pick,
}

enum TargetChoice {
    Claude,
    Codex(CodexStart),
}

enum ChosenModel {
    Claude(Option<String>),
    Codex(String),
}

pub async fn prepare(request: &mut Preparation<'_>) -> Result<PreparedSetup, AgentSetupError> {
    let default = default_model(request);
    let choice = target_choice(request, default.clone())?;
    let connected = connect_and_list(request, default.as_deref()).await?;
    let chosen = choose_model(
        request,
        &connected.listing,
        default,
        connected.repair_model,
        choice,
    )?;
    let model = match chosen {
        ChosenModel::Claude(unpinned) => {
            let validated =
                validated_model(unpinned.as_deref(), &connected.listing, &connected.base_url)?;
            source_name(&connected.listing, validated)
        }
        ChosenModel::Codex(model_name) => {
            validated_model(Some(&model_name), &connected.listing, &connected.base_url)?;
            Some(model_name)
        }
    };
    let settings_path = resolved_settings_path(request.target, request.environment)?;
    let saved = SavedSetup::new(
        request.target,
        settings_path,
        connected.base_url,
        connected.credential.token,
        model,
    )
    .map_err(|source| AgentSetupError::InvalidSetup {
        source: Box::new(source),
    })?;
    Ok(PreparedSetup {
        saved,
        listing: connected.listing,
    })
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

fn target_choice(
    request: &Preparation<'_>,
    default: Option<String>,
) -> Result<TargetChoice, AgentSetupError> {
    match request.target {
        Target::Claude => Ok(TargetChoice::Claude),
        Target::Codex => Ok(TargetChoice::Codex(codex_start(request, default)?)),
    }
}

fn codex_start(
    request: &Preparation<'_>,
    default: Option<String>,
) -> Result<CodexStart, AgentSetupError> {
    if request.mode.interactive {
        return Ok(CodexStart::Pick);
    }
    default.map_or_else(
        || Err(AgentSetupError::Usage(CODEX_MODEL_REQUIRED.to_owned())),
        |model| Ok(CodexStart::Ready(model)),
    )
}

fn choose_model(
    request: &mut Preparation<'_>,
    listing: &[ListedModel],
    default: Option<String>,
    repair_model: bool,
    choice: TargetChoice,
) -> Result<ChosenModel, AgentSetupError> {
    match choice {
        TargetChoice::Claude => {
            let chosen = if request.mode.interactive || repair_model {
                let names: Vec<String> = listing
                    .iter()
                    .map(|listed| {
                        listed
                            .source_model
                            .clone()
                            .unwrap_or_else(|| listed.id.clone())
                    })
                    .collect();
                pick_claude_model(request.prompter, &names, default.as_deref())?
            } else {
                default
            };
            Ok(ChosenModel::Claude(chosen))
        }
        TargetChoice::Codex(start) => {
            let chosen = match start {
                CodexStart::Ready(model) if !repair_model => model,
                CodexStart::Ready(_) | CodexStart::Pick => {
                    let ids: Vec<String> = listing.iter().map(|listed| listed.id.clone()).collect();
                    pick_codex_model(request.prompter, &ids, default.as_deref())?
                }
            };
            Ok(ChosenModel::Codex(chosen))
        }
    }
}

struct Connected {
    base_url: String,
    credential: StaticToken,
    listing: Vec<ListedModel>,
    repair_model: bool,
}

async fn connect_and_list(
    request: &mut Preparation<'_>,
    default: Option<&str>,
) -> Result<Connected, AgentSetupError> {
    let (base_url, credential) = connect(request)?;
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
    let repair_model =
        repair && default.is_some_and(|model| starting_model(model, &listing).is_none());
    Ok(Connected {
        base_url,
        credential: active,
        listing,
        repair_model,
    })
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
