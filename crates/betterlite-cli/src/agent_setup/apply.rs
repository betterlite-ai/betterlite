use std::io::Write;
use std::path::Path;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::files::PrivateFiles;
use regex::Regex;
use soft_canonicalize::soft_canonicalize;

use super::error::AgentSetupError;
use super::listing::validated_model;
use crate::agent_launch::codex_config_path;
use crate::agent_setup_profiles::error::AgentSetupProfilesError;
use crate::agent_setup_profiles::{SavedSetup, Target, save_setup};
use crate::claude_settings::choices::{ModelChoice, STARTING_MODEL_ROLE, StaticToken};
use crate::claude_settings::paths::{
    claude_settings_path, claude_settings_paths, configure_state_path, settings_file_owners,
};
use crate::claude_settings::receipts::configure::{ConfigureRequest, configure_claude_settings};
use crate::codex_settings::configure::{ConfigureCodexSettings, configure_codex_settings};
use crate::codex_settings::version::CodexVersion;
use crate::pi::ListedModel;

pub const CLAUDE_PICKER_PATTERN: &str = "(?i)claude|anthropic";

pub struct ApplyRequest<'request> {
    pub saved: &'request SavedSetup,
    pub listing: &'request [ListedModel],
    pub environment: &'request (dyn EnvironmentLookup + Send + Sync),
    pub codex_version: CodexVersion<'request>,
    pub stdout: &'request mut (dyn Write + Send),
    pub stderr: &'request mut (dyn Write + Send),
}

const fn target_name(target: Target) -> &'static str {
    match target {
        Target::Claude => "Claude",
        Target::Codex => "Codex",
    }
}

pub async fn apply_setup(request: &mut ApplyRequest<'_>) -> Result<(), AgentSetupError> {
    save_setup(request.saved, request.environment).await?;
    let target = request.saved.target;
    let applied = match target {
        Target::Claude => apply_claude(request).await,
        Target::Codex => match request.saved.model.as_deref() {
            Some(model) => apply_codex(request, model).await,
            None => Ok(()),
        },
    };
    applied.map_err(|source| AgentSetupError::Apply {
        name: target_name(target),
        target,
        source: Box::new(source),
    })?;
    writeln!(
        request.stdout,
        "Setup saved. Edit with `lite reconfigure {target}`; remove saved settings and key with `lite unconfigure {target} --forget`."
    )?;
    Ok(())
}

fn write_symlink_note(
    request: &mut ApplyRequest<'_>,
    settings_path: &Path,
    target: Target,
) -> Result<(), AgentSetupError> {
    if !settings_path.is_symlink() {
        return Ok(());
    }
    let resolved = soft_canonicalize(settings_path).map_err(|source| AgentSetupError::Resolve {
        path: settings_path.to_owned(),
        source,
    })?;
    let note = match target {
        Target::Claude => format!(
            "Note: {} is a symlink to {}, so your key now lives in that file; keep it out of version control.",
            settings_path.display(),
            resolved.display()
        ),
        Target::Codex => format!(
            "Note: your key now lives in {}; keep it out of version control.",
            resolved.display()
        ),
    };
    writeln!(request.stderr, "{note}")?;
    Ok(())
}

async fn apply_claude(request: &mut ApplyRequest<'_>) -> Result<(), AgentSetupError> {
    let saved = request.saved;
    let starting = validated_model(saved.model.as_deref(), request.listing, &saved.base_url)?;
    let settings_path = claude_settings_path(request.environment).ok_or_else(|| {
        AgentSetupError::Profile(Box::new(AgentSetupProfilesError::Path(Target::Claude)))
    })?;
    configure_claude(request.saved, request.environment, starting, &settings_path).await?;
    let pattern =
        Regex::new(CLAUDE_PICKER_PATTERN).map_err(|source| AgentSetupError::Pattern { source })?;
    print_claude(request, starting, &settings_path, &pattern)?;
    write_symlink_note(request, &settings_path, Target::Claude)
}

async fn configure_claude(
    saved: &SavedSetup,
    environment: &(dyn EnvironmentLookup + Send + Sync),
    starting: Option<&str>,
    settings_path: &Path,
) -> Result<(), AgentSetupError> {
    let home = environment.system_home().ok_or_else(|| {
        AgentSetupError::Profile(Box::new(AgentSetupProfilesError::Path(Target::Claude)))
    })?;
    let paths = claude_settings_paths(&home);
    let state_path = configure_state_path(settings_path, &paths)?;
    let owners = settings_file_owners(settings_path, &paths)?;
    let credential = StaticToken {
        token: saved.api_key.clone(),
    };
    let model = starting.map_or(ModelChoice::UnpinModel, |id| {
        ModelChoice::StartOn(id.to_owned())
    });
    let files = PrivateFiles::default();
    let configure = ConfigureRequest {
        base_url: &saved.base_url,
        credential: &credential,
        model: &model,
        settings_path,
        state_path: &state_path,
        owners: &owners,
        binary_path: None,
    };
    configure_claude_settings(&configure, &files, &files).await?;
    Ok(())
}

fn print_claude(
    request: &mut ApplyRequest<'_>,
    starting: Option<&str>,
    settings_path: &Path,
    pattern: &Regex,
) -> Result<(), AgentSetupError> {
    let listed = request.listing.len();
    let in_picker = request
        .listing
        .iter()
        .filter(|model| pattern.is_match(&model.id))
        .count();
    writeln!(
        request.stdout,
        "Configured Claude Code: {} now routes through {}.",
        settings_path.display(),
        request.saved.base_url
    )?;
    writeln!(
        request.stdout,
        "Credential: your virtual key, stored in the file as ANTHROPIC_AUTH_TOKEN."
    )?;
    let starting_line = starting_line(starting);
    writeln!(request.stdout, "{starting_line}")?;
    let picker_line = picker_line(listed, in_picker);
    writeln!(request.stdout, "{picker_line}")?;
    writeln!(
        request.stdout,
        "Start `claude` from any terminal. Undo with `lite unconfigure claude`."
    )?;
    Ok(())
}

fn starting_line(starting: Option<&str>) -> String {
    starting.map_or_else(
        || "Starting model: not pinned (Claude Code's default, or a model you set yourself); switch with /model, or pass --model to start on a proxy model. Without a pin, a resumed session re-sends the model its transcript recorded, which behind a raw-model auto-router is the tier model.".to_owned(),
        |model| format!(
            "Starting model: {model} ({STARTING_MODEL_ROLE}); switch any time with /model."
        ),
    )
}

fn picker_line(listed: usize, in_picker: usize) -> String {
    if in_picker == listed {
        return format!("/model will list all {listed} of the proxy's models.");
    }
    format!(
        "/model will list {in_picker} of the proxy's {listed} models: Claude Code shows only ids containing 'claude' or 'anthropic', and this proxy does not list the rest under such names."
    )
}

fn print_codex(
    request: &mut ApplyRequest<'_>,
    model: &str,
    settings_path: &Path,
) -> Result<(), AgentSetupError> {
    writeln!(
        request.stdout,
        "Configured Codex: {} now routes through {}.",
        settings_path.display(),
        request.saved.base_url
    )?;
    writeln!(
        request.stdout,
        "Starting model: {model}. Credential: your virtual key, stored in the private provider settings."
    )?;
    writeln!(
        request.stdout,
        "Start `codex` from any terminal. Undo with `lite unconfigure codex`."
    )?;
    Ok(())
}

async fn apply_codex(request: &mut ApplyRequest<'_>, model: &str) -> Result<(), AgentSetupError> {
    validated_model(Some(model), request.listing, &request.saved.base_url)?;
    let settings_path = codex_config_path(request.environment).ok_or_else(|| {
        AgentSetupError::Profile(Box::new(AgentSetupProfilesError::Path(Target::Codex)))
    })?;
    let configure = ConfigureCodexSettings {
        base_url: &request.saved.base_url,
        api_key: &request.saved.api_key,
        model,
        settings_path: &settings_path,
        version: request.codex_version,
    };
    configure_codex_settings(&configure, &PrivateFiles::default()).await?;
    print_codex(request, model, &settings_path)?;
    write_symlink_note(request, &settings_path, Target::Codex)
}
