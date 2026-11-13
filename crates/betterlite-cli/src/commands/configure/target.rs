use crate::agent_setup::prepare::SetupMode;
use crate::agent_setup::{ConfigureTargets, configure_targets};
use crate::agent_setup_profiles::Target;
use crate::agent_setup_profiles::locks::setup_locks;
use crate::cli_context::CliContext;

use super::SetupPorts;
use super::args::{ConnectionFlags, GroupConnection};
use super::connection::connection_settings;
use super::error::ConfigureCommandError;

pub const APPLY: SetupMode = SetupMode {
    interactive: false,
    edit_connection: false,
};

pub const WIZARD: SetupMode = SetupMode {
    interactive: true,
    edit_connection: false,
};

pub const EDIT_WIZARD: SetupMode = SetupMode {
    interactive: true,
    edit_connection: true,
};

pub struct TargetRequest<'request> {
    pub target: Target,
    pub connection: &'request ConnectionFlags,
    pub group: &'request GroupConnection,
    pub model: Option<&'request str>,
    pub default_model: bool,
    pub edit: bool,
}

pub async fn configure_target(
    request: &TargetRequest<'_>,
    ports: &mut SetupPorts<'_>,
) -> Result<(), ConfigureCommandError> {
    let settings = connection_settings(
        ports.context,
        Some(request.group),
        request.connection.api_key.as_ref(),
        request.connection.gateway_url.as_deref(),
    )?;
    let interactive = request.edit && request.model.is_none() && !request.default_model;
    if interactive && !ports.stdin_is_terminal {
        return Err(ConfigureCommandError::Terminal {
            command: "reconfigure",
        });
    }
    let _locks = setup_locks(&[request.target], ports.environment, ports.clock).await?;
    configure_targets(ConfigureTargets {
        context: &settings,
        targets: &[request.target],
        model: request.model,
        default_model: request.default_model,
        mode: SetupMode {
            interactive,
            edit_connection: interactive,
        },
        environment: ports.environment,
        prompter: &mut *ports.prompter,
        stdin_is_terminal: ports.stdin_is_terminal,
        codex_version: ports.codex_version,
        stdout: &mut *ports.stdout,
        stderr: &mut *ports.stderr,
    })
    .await?;
    Ok(())
}

pub async fn apply_mode(
    context: &CliContext,
    targets: &[Target],
    mode: SetupMode,
    ports: &mut SetupPorts<'_>,
) -> Result<(), ConfigureCommandError> {
    configure_targets(ConfigureTargets {
        context,
        targets,
        model: None,
        default_model: false,
        mode,
        environment: ports.environment,
        prompter: &mut *ports.prompter,
        stdin_is_terminal: ports.stdin_is_terminal,
        codex_version: ports.codex_version,
        stdout: &mut *ports.stdout,
        stderr: &mut *ports.stderr,
    })
    .await?;
    Ok(())
}
