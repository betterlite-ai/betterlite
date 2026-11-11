pub mod args;
pub mod connection;
pub mod disconnect;
pub mod error;
pub mod group;
pub mod target;

use std::io::Write;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::files::PrivateFiles;

use crate::agent_setup::prompt::Prompter;
use crate::agent_setup_profiles::Target;
use crate::agent_setup_profiles::locks::Clock;
use crate::cli_context::CliContext;
use crate::codex_settings::version::CodexVersion;

use args::{
    ConfigureArgs, ConfigureTarget, ReconfigureArgs, ReconfigureTarget, UnconfigureArgs,
    UnconfigureTarget,
};
use error::ConfigureCommandError;
use target::TargetRequest;

#[path = "configure/apply_saved_setup/tests.rs"]
mod apply_saved_setup;
#[path = "configure/forget/tests.rs"]
mod forget;
#[path = "configure/missing_receipt/tests.rs"]
mod missing_receipt;
#[path = "configure/option_precedence/tests.rs"]
mod option_precedence;
#[path = "configure/reconfigure/tests.rs"]
mod reconfigure;
#[path = "configure/terminal_requirement/tests.rs"]
mod terminal_requirement;
#[path = "configure/tests/support.rs"]
mod test_support;
#[path = "configure/withheld_credentials/tests.rs"]
mod withheld_credentials;

pub const TARGETS: [Target; 2] = [Target::Claude, Target::Codex];

pub struct SetupPorts<'ports> {
    pub context: &'ports CliContext,
    pub environment: &'ports (dyn EnvironmentLookup + Send + Sync),
    pub files: &'ports PrivateFiles,
    pub prompter: &'ports mut dyn Prompter,
    pub stdin_is_terminal: bool,
    pub clock: &'ports dyn Clock,
    pub codex_version: CodexVersion<'ports>,
    pub stdout: &'ports mut (dyn Write + Send),
    pub stderr: &'ports mut (dyn Write + Send),
}

pub async fn run_configure(
    args: ConfigureArgs,
    ports: &mut SetupPorts<'_>,
) -> Result<(), ConfigureCommandError> {
    let ConfigureArgs {
        connection: group,
        command,
    } = args;
    let Some(selected) = command else {
        return group::configure_group(&group, false, ports).await;
    };
    let (target, flags, model, default_model) = match selected {
        ConfigureTarget::Claude {
            connection,
            model,
            default_model,
        } => (Target::Claude, connection, model, default_model),
        ConfigureTarget::Codex { connection, model } => (Target::Codex, connection, model, false),
    };
    let request = TargetRequest {
        target,
        connection: &flags,
        group: &group,
        model: model.as_deref(),
        default_model,
        edit: false,
    };
    target::configure_target(&request, ports).await
}

pub async fn run_reconfigure(
    args: ReconfigureArgs,
    ports: &mut SetupPorts<'_>,
) -> Result<(), ConfigureCommandError> {
    let ReconfigureArgs {
        connection: group,
        command,
    } = args;
    let Some(selected) = command else {
        return group::configure_group(&group, true, ports).await;
    };
    let (target, flags, model, default_model) = match selected {
        ReconfigureTarget::Claude {
            connection,
            model,
            default_model,
        } => (Target::Claude, connection, model, default_model),
        ReconfigureTarget::Codex { connection, model } => (Target::Codex, connection, model, false),
    };
    let request = TargetRequest {
        target,
        connection: &flags,
        group: &group,
        model: model.as_deref(),
        default_model,
        edit: true,
    };
    target::configure_target(&request, ports).await
}

pub async fn run_unconfigure(
    args: UnconfigureArgs,
    ports: &mut SetupPorts<'_>,
) -> Result<(), ConfigureCommandError> {
    let UnconfigureArgs { forget, command } = args;
    match command {
        None => disconnect::unconfigure_group(forget, ports).await,
        Some(UnconfigureTarget::Claude {
            forget: target_forget,
        }) => disconnect::unconfigure_target(Target::Claude, forget || target_forget, ports).await,
        Some(UnconfigureTarget::Codex {
            forget: target_forget,
        }) => disconnect::unconfigure_target(Target::Codex, forget || target_forget, ports).await,
    }
}
