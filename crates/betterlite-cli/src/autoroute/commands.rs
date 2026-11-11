pub mod args;
pub mod configure;
pub mod entropy;
pub mod error;
pub mod liveliness;
pub mod master_key;
pub mod signals;
pub mod start;
pub mod stop;
pub mod teardown;

use std::io::Write;
use std::path::Path;

use betterlite_logging::private_json::files::PrivateFiles;

use self::args::AutorouteCommand;
use self::entropy::Entropy;
use self::error::CommandsError;
use self::signals::StopSignals;
use crate::autoroute::process::control::{Clock, SignalPort};
use crate::autoroute::process::launcher::ProcessLauncher;
use crate::autoroute::process::liveliness::LivelinessProbe;
use crate::autoroute::process::log_tail::LogTail;
use crate::autoroute::process::port::PortProbe;
use crate::autoroute::process::runtime::GatewayRuntime;
use crate::autoroute::wizard::{ModelDiscoveryClient, Prompter};
use crate::cli_context::CliContext;

pub struct AutoroutePorts<'ports> {
    pub context: &'ports CliContext,
    pub home: &'ports Path,
    pub client: &'ports dyn ModelDiscoveryClient,
    pub prompter: &'ports mut dyn Prompter,
    pub stdin_is_terminal: bool,
    pub gateway: &'ports dyn GatewayRuntime,
    pub launcher: &'ports dyn ProcessLauncher,
    pub port_probe: &'ports dyn PortProbe,
    pub liveliness: &'ports dyn LivelinessProbe,
    pub signals: &'ports dyn StopSignals,
    pub signal_port: &'ports dyn SignalPort,
    pub tail: &'ports dyn LogTail,
    pub clock: &'ports dyn Clock,
    pub files: &'ports PrivateFiles,
    pub entropy: &'ports mut dyn Entropy,
    pub binary_path: Option<&'ports Path>,
    pub stdout: &'ports mut dyn Write,
    pub stderr: &'ports mut dyn Write,
}

pub fn run(command: AutorouteCommand, ports: &mut AutoroutePorts<'_>) -> Result<(), CommandsError> {
    match command {
        AutorouteCommand::Configure => configure::configure(ports),
        AutorouteCommand::Start(args) => start::start(ports, args.port),
        AutorouteCommand::Stop => stop::stop(ports),
        AutorouteCommand::Up(args) => {
            writeln!(ports.stderr, "{}", deprecation_notice("up", "start"))
                .map_err(|source| CommandsError::Output { source })?;
            start::start(ports, args.port)
        }
        AutorouteCommand::Down => {
            writeln!(ports.stderr, "{}", deprecation_notice("down", "stop"))
                .map_err(|source| CommandsError::Output { source })?;
            stop::stop(ports)
        }
    }
}

#[must_use]
pub fn deprecation_notice(retired: &str, current: &str) -> String {
    format!(
        "`lite autoroute {retired}` is deprecated and will be removed in a future release; run \
         `lite autoroute {current}` instead, it takes the same options."
    )
}

mod tests;
