pub mod apply;
pub mod connection;
pub mod error;
pub mod listing;
pub mod pickers;
pub mod preflight;
pub mod prepare;
pub mod prompt;

use std::io::Write;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};

use crate::agent_setup_profiles::{Target, read_saved_setup};
use crate::claude_settings::choices::StaticToken;
use crate::cli_context::CliContext;
use crate::codex_settings::version::CodexVersion;

use apply::ApplyRequest;
use error::AgentSetupError;
use pickers::{EMPTY_TARGETS_MESSAGE, TARGET_EDIT_MESSAGE, TARGET_ROUTE_MESSAGE};
use prepare::{Preparation, SetupMode, prepare};
use prompt::Prompter;

pub const MODEL_OPTION_HELP: &str = "Proxy model to set as the /model picker's default row, the model Claude Code starts and resumes on. Must be listed on /v1/models for the key; omission keeps the saved choice. Use --default-model to stop pinning a model. Nothing pins Claude Code's sub-agent or background tiers; `lite autoroute start` is the mode that does.";

pub struct ConfigureTargets<'setup> {
    pub context: &'setup CliContext,
    pub targets: &'setup [Target],
    pub model: Option<&'setup str>,
    pub default_model: bool,
    pub mode: SetupMode,
    pub environment: &'setup (dyn EnvironmentLookup + Send + Sync),
    pub prompter: &'setup mut dyn Prompter,
    pub stdin_is_terminal: bool,
    pub codex_version: CodexVersion<'setup>,
    pub stdout: &'setup mut (dyn Write + Send),
    pub stderr: &'setup mut (dyn Write + Send),
}

pub fn resolve_credential(
    context: &CliContext,
    api_key: Option<&SecretString>,
) -> Result<StaticToken, AgentSetupError> {
    let explicit = match api_key {
        Some(key) => Some(key),
        None if context.api_key_from_token_file => None,
        None => context.api_key.as_ref(),
    };
    let Some(key) = explicit else {
        return Err(AgentSetupError::MissingCredential);
    };
    let text = key.expose_secret();
    if text.trim().is_empty()
        || text
            .chars()
            .any(|character| character <= ' ' || character == '\u{7f}')
    {
        return Err(AgentSetupError::InvalidCredential);
    }
    Ok(StaticToken { token: key.clone() })
}

pub fn pick_targets(
    prompter: &mut dyn Prompter,
    defaults: &[Target],
    edit: bool,
) -> Result<Vec<Target>, AgentSetupError> {
    let message = if edit {
        TARGET_EDIT_MESSAGE
    } else {
        TARGET_ROUTE_MESSAGE
    };
    loop {
        let picked = prompter.pick_targets(message, defaults, EMPTY_TARGETS_MESSAGE)?;
        if !picked.is_empty() {
            return Ok([Target::Claude, Target::Codex]
                .into_iter()
                .filter(|target| picked.contains(target))
                .collect());
        }
    }
}

pub async fn configure_targets(request: ConfigureTargets<'_>) -> Result<(), AgentSetupError> {
    if request.model.is_some() && request.default_model {
        return Err(AgentSetupError::Usage(
            "--model and --default-model cannot be used together".to_owned(),
        ));
    }
    for target in request.targets {
        preflight::preflight(*target, request.environment, request.codex_version).await?;
    }
    let mut setups = Vec::new();
    for target in request.targets {
        let saved = read_saved_setup(*target, request.environment)?;
        let mut preparation = Preparation {
            target: *target,
            saved,
            model: request.model.map(str::to_owned),
            default_model: request.default_model,
            mode: request.mode,
            context: request.context,
            environment: request.environment,
            prompter: &mut *request.prompter,
            stdin_is_terminal: request.stdin_is_terminal,
        };
        setups.push(prepare(&mut preparation).await?);
    }
    for setup in setups {
        let mut apply_request = ApplyRequest {
            saved: &setup.saved,
            listing: &setup.listing,
            environment: request.environment,
            codex_version: request.codex_version,
            stdout: &mut *request.stdout,
            stderr: &mut *request.stderr,
        };
        apply::apply_setup(&mut apply_request).await?;
    }
    Ok(())
}

mod tests;
