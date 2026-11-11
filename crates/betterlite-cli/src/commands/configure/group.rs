use betterlite_config::loader::EnvironmentLookup;

use crate::agent_setup::pick_targets;
use crate::agent_setup_profiles::Target;
use crate::agent_setup_profiles::locks::setup_locks;
use crate::agent_setup_profiles::read_saved_setup;

use super::SetupPorts;
use super::TARGETS;
use super::args::GroupConnection;
use super::connection::connection_settings;
use super::error::ConfigureCommandError;
use super::target::{APPLY, EDIT_WIZARD, WIZARD, apply_mode};

const GATEWAY_URL_PROMPT: &str = "Gateway URL";

pub async fn configure_group(
    connection: &GroupConnection,
    edit: bool,
    ports: &mut SetupPorts<'_>,
) -> Result<(), ConfigureCommandError> {
    let settings = connection_settings(
        ports.context,
        None,
        connection.api_key.as_ref(),
        connection.gateway_url.as_deref(),
    )?;
    let _locks = setup_locks(&TARGETS, ports.environment, ports.clock).await?;
    let saved = saved_targets(ports.environment)?;
    if !saved.is_empty() && !edit {
        return apply_mode(&settings, &saved, APPLY, ports).await;
    }
    if !ports.stdin_is_terminal {
        return Err(ConfigureCommandError::Terminal {
            command: if edit { "reconfigure" } else { "configure" },
        });
    }
    let defaults: &[Target] = if saved.is_empty() { &TARGETS } else { &saved };
    let selected = pick_targets(&mut *ports.prompter, defaults, edit)?;
    if edit {
        return apply_mode(&settings, &selected, EDIT_WIZARD, ports).await;
    }
    let context = if settings.base_url_explicit {
        settings
    } else {
        let entered = ports
            .prompter
            .prompt_url(GATEWAY_URL_PROMPT, &settings.base_url)?;
        connection_settings(&settings, None, None, Some(&entered))?
    };
    apply_mode(&context, &selected, WIZARD, ports).await
}

fn saved_targets(
    environment: &(dyn EnvironmentLookup + Send + Sync),
) -> Result<Vec<Target>, ConfigureCommandError> {
    let mut saved = Vec::new();
    for target in TARGETS {
        if read_saved_setup(target, environment)?.is_some() {
            saved.push(target);
        }
    }
    Ok(saved)
}
