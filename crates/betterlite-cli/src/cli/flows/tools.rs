use betterlite_logging::private_json::files::PrivateFiles;
use tokio::runtime::Builder;

use crate::agent_setup::prompt::InquirePrompter as SetupPrompter;
use crate::agent_setup_profiles::locks::SystemClock;
use crate::autoroute::commands::args::AutorouteArgs;
use crate::autoroute::commands::entropy::OsEntropy;
use crate::autoroute::commands::liveliness::HttpLiveliness;
use crate::autoroute::commands::signals::OsSignals;
use crate::autoroute::commands::{AutoroutePorts, run as run_autoroute};
use crate::autoroute::process::control::{PathSignals, RealClock};
use crate::autoroute::process::launcher::GatewayLauncher;
use crate::autoroute::process::log_tail::StdoutLogTail;
use crate::autoroute::process::port::LoopbackPortProbe;
use crate::autoroute::process::runtime::InstalledGateway;
use crate::autoroute::wizard::prompt::InquirePrompter as WizardPrompter;
use crate::codex_settings::version::CodexVersion;
use crate::commands::Client;
use crate::commands::configure::{AgentSetupCommand, SetupPorts, run as run_setup};

use super::super::Failure;
use super::super::ports::wiring::path_resolver;
use super::super::session::Session;

pub(crate) fn configure(
    session: &mut Session<'_>,
    command: AgentSetupCommand,
) -> Result<(), Failure> {
    let private_files = PrivateFiles::default();
    let mut prompter = SetupPrompter;
    let clock = SystemClock;
    let mut ports = SetupPorts {
        context: &session.context,
        environment: session.environment,
        files: &private_files,
        prompter: &mut prompter,
        stdin_is_terminal: session.stdin_is_terminal,
        clock: &clock,
        codex_version: CodexVersion::Installed,
        stdout: &mut session.streams.stdout,
        stderr: &mut session.streams.stderr,
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| Failure::Plain(format!("the async runtime could not start: {source}")))?;
    runtime.block_on(run_setup(command, &mut ports))?;
    Ok(())
}

pub(crate) fn autoroute(session: &mut Session<'_>, args: AutorouteArgs) -> Result<(), Failure> {
    let home = session
        .home
        .clone()
        .ok_or_else(|| Failure::Plain(String::from("cannot find your home directory")))?;
    let private_files = PrivateFiles::default();
    let client: &Client = &session.client;
    let liveliness = HttpLiveliness { http: client };
    let mut prompter = WizardPrompter;
    let gateway = InstalledGateway {
        resolver: path_resolver(session),
    };
    let mut entropy = OsEntropy;
    let mut ports = AutoroutePorts {
        context: &session.context,
        home: &home,
        client,
        prompter: &mut prompter,
        stdin_is_terminal: session.stdin_is_terminal,
        gateway: &gateway,
        launcher: &GatewayLauncher,
        port_probe: &LoopbackPortProbe,
        liveliness: &liveliness,
        signals: &OsSignals,
        signal_port: &PathSignals,
        tail: &StdoutLogTail,
        clock: &RealClock,
        files: &private_files,
        entropy: &mut entropy,
        binary_path: session.binary_path.as_deref(),
        stdout: &mut session.streams.stdout,
        stderr: &mut session.streams.stderr,
    };
    run_autoroute(args.command, &mut ports)?;
    Ok(())
}
