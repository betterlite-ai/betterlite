use std::io;
use std::io::Write;
use std::path::Path;

use crate::agent_setup_profiles::Target;
use crate::agent_setup_profiles::error::AgentSetupProfilesError;
use crate::agent_setup_profiles::locks::setup_locks;
use crate::agent_setup_profiles::{
    forget_saved_setup, receipt_path_for, settings_path_for, setup_profile_path,
};
use crate::claude_settings::BASE_URL_PATH;
use crate::claude_settings::paths::{claude_settings_paths, settings_file_owners};
use crate::claude_settings::receipts::preflight::preflight_claude_settings;
use crate::claude_settings::receipts::unconfigure::{
    UnconfigureOutcome, WithheldCredential, unconfigure_claude_settings,
};
use crate::codex_settings::unconfigure::unconfigure_codex_settings;

use super::SetupPorts;
use super::TARGETS;
use super::error::ConfigureCommandError;

pub async fn unconfigure_group(
    forget: bool,
    ports: &mut SetupPorts<'_>,
) -> Result<(), ConfigureCommandError> {
    let _locks = setup_locks(&TARGETS, ports.environment, ports.clock).await?;
    for target in TARGETS {
        disconnect(target, forget, ports).await?;
    }
    Ok(())
}

pub async fn unconfigure_target(
    target: Target,
    forget: bool,
    ports: &mut SetupPorts<'_>,
) -> Result<(), ConfigureCommandError> {
    let _locks = setup_locks(&[target], ports.environment, ports.clock).await?;
    disconnect(target, forget, ports).await
}

async fn disconnect(
    target: Target,
    forget: bool,
    ports: &mut SetupPorts<'_>,
) -> Result<(), ConfigureCommandError> {
    let settings_path = settings_path_for(target, ports.environment)
        .ok_or(AgentSetupProfilesError::Path(target))?;
    let state_path = receipt_path_for(target, &settings_path, ports.environment)?;
    let profile = setup_profile_path(target, &settings_path, ports.environment)?;
    if state_path.exists() {
        match target {
            Target::Claude => undo_claude(&settings_path, &state_path, ports).await?,
            Target::Codex => undo_codex(&settings_path, ports).await?,
        }
    } else {
        missing_receipt(target, &settings_path, &state_path, ports)?;
    }
    if forget {
        forget_saved_setup(target, ports.environment)?;
        writeln!(
            ports.stdout,
            "Forgot saved {target} setup, including its saved key."
        )?;
    }
    if !forget && profile.exists() {
        writeln!(
            ports.stdout,
            "Saved setup retained. Run `lite configure {target}` to apply it again."
        )?;
    }
    Ok(())
}

fn missing_receipt(
    target: Target,
    settings_path: &Path,
    state_path: &Path,
    ports: &mut SetupPorts<'_>,
) -> Result<(), ConfigureCommandError> {
    writeln!(
        ports.stdout,
        "No {target} undo receipt at {}; nothing to undo. Agent settings were not changed.",
        state_path.display()
    )?;
    if settings_path.exists() {
        writeln!(
            ports.stderr,
            "Cannot confirm disconnection. Check {} and remove any remaining gateway connection and key manually.",
            settings_path.display()
        )?;
    }
    Ok(())
}

async fn undo_claude(
    settings_path: &Path,
    state_path: &Path,
    ports: &mut SetupPorts<'_>,
) -> Result<(), ConfigureCommandError> {
    let home = ports
        .environment
        .system_home()
        .ok_or(AgentSetupProfilesError::Path(Target::Claude))?;
    let paths = claude_settings_paths(&home);
    preflight_claude_settings(settings_path, &paths)?;
    let owners = settings_file_owners(settings_path, &paths)?;
    let outcome =
        unconfigure_claude_settings(settings_path, state_path, &owners, ports.files, ports.files)
            .await?;
    report_unconfigure(settings_path, state_path, &outcome, &mut *ports.stdout)?;
    Ok(())
}

async fn undo_codex(
    settings_path: &Path,
    ports: &mut SetupPorts<'_>,
) -> Result<(), ConfigureCommandError> {
    let outcome = unconfigure_codex_settings(settings_path, ports.files).await?;
    if outcome.file_removed {
        writeln!(
            ports.stdout,
            "Removed {}; it held only settings created by `lite configure codex`.",
            settings_path.display()
        )?;
    } else if outcome.restored.is_empty() {
        writeln!(
            ports.stdout,
            "Nothing in {} was still ours to restore.",
            settings_path.display()
        )?;
    } else {
        writeln!(
            ports.stdout,
            "Restored in {}: {}.",
            settings_path.display(),
            outcome.restored.join(", ")
        )?;
    }
    if !outcome.kept.is_empty() {
        writeln!(
            ports.stdout,
            "Left as you changed them since: {}.",
            outcome.kept.join(", ")
        )?;
    }
    Ok(())
}

fn report_unconfigure(
    settings_path: &Path,
    state_path: &Path,
    outcome: &UnconfigureOutcome,
    stdout: &mut dyn Write,
) -> io::Result<()> {
    if outcome.file_removed {
        writeln!(
            stdout,
            "No settings file remains at {}; it held nothing but `lite configure claude`'s own keys.",
            settings_path.display()
        )?;
    } else if outcome.restored.is_empty() {
        writeln!(
            stdout,
            "Nothing in {} was still ours to restore.",
            settings_path.display()
        )?;
    } else {
        writeln!(
            stdout,
            "Restored in {}: {}.",
            settings_path.display(),
            outcome.restored.join(", ")
        )?;
    }
    if !outcome.kept.is_empty() {
        writeln!(
            stdout,
            "Left as you changed them since: {}.",
            outcome.kept.join(", ")
        )?;
    }
    if outcome.withheld.is_empty() {
        return Ok(());
    }
    write_withheld(&outcome.withheld, state_path, stdout)
}

fn write_withheld(
    withheld: &[WithheldCredential],
    state_path: &Path,
    stdout: &mut dyn Write,
) -> io::Result<()> {
    let captured: Vec<String> = withheld
        .iter()
        .map(|item| format!("{} (captured with {})", item.key, item.endpoint))
        .collect();
    writeln!(
        stdout,
        "Left removed, since the file now points at a different server than they were issued for: {}. They stay in {}: point {BASE_URL_PATH} back and run `lite unconfigure claude` again to put them back, or delete that file to drop them.",
        captured.join("; "),
        state_path.display()
    )
}
