use std::sync::Arc;

use betterlite_logging::private_json::files::PrivateFiles;
use tokio::runtime::Builder;

use crate::agent_setup::prompt::InquirePrompter as SetupPrompter;
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
use crate::login::ports::{FlowPorts, LoginPorts};
use crate::login::{LoginArgs, run_login};
use crate::sso_polling::terminal::Terminal as SelectionTerminal;
#[cfg(unix)]
use crate::up::command::run_up;
#[cfg(unix)]
use crate::up::ports::{FlowLogin, UpPorts};
#[cfg(unix)]
use crate::up::signals::SignalStop;

use super::Failure;
use super::ports::wiring::{Wiring, message_sink, no_browser, path_resolver};
use super::session::{Session, SystemEntropy};

pub(crate) fn login(session: &mut Session<'_>, args: LoginArgs) -> Result<(), Failure> {
    let mut wiring = Wiring::new(session)?;
    let mut entropy = SystemEntropy;
    let mut selection = SelectionTerminal::default();
    let clock = Arc::clone(&session.clock);
    let mut stdout = session.streams.stdout.clone();
    let mut ports = LoginPorts {
        flow: FlowPorts {
            context: &session.context,
            environment: session.environment,
            files: &wiring.files,
            vault: &mut *session.vault,
            private_files: &wiring.private_files,
            http: Arc::clone(&session.client),
            clock: &*clock,
            wall_clock: &*clock,
            entropy: &mut entropy,
            sleeper: &wiring.sleeper,
            selection: &mut selection,
            browser: &no_browser,
            interrupt: &wiring.interrupt,
            token_path: &wiring.token_path,
            binary_path: session.binary_path.as_deref(),
        },
        stdout: &mut stdout,
    };
    Ok(run_login(args, &mut ports)?)
}

#[cfg(unix)]
pub(crate) fn up(session: &mut Session<'_>) -> Result<u8, Failure> {
    let mut wiring = Wiring::new(session)?;
    let mut entropy = SystemEntropy;
    let mut selection = SelectionTerminal::default();
    let mut login = FlowLogin;
    let signals = SignalStop::default();
    let warn = message_sink(session.streams.stderr.clone());
    let mut stdout = session.streams.stdout.clone();
    let mut stderr = session.streams.stderr.clone();
    let mut ports = UpPorts {
        context: &session.context,
        environment: session.environment,
        files: &wiring.files,
        vault: &mut *session.vault,
        private_files: &wiring.private_files,
        http: Arc::clone(&session.client),
        clock: Arc::clone(&session.clock),
        entropy: &mut entropy,
        sleeper: &wiring.sleeper,
        selection: &mut selection,
        browser: &no_browser,
        interrupt: &wiring.interrupt,
        token_path: &wiring.token_path,
        binary_path: session.binary_path.as_deref(),
        home: session.home.as_deref(),
        expiration_hours: session.expiration_hours,
        warn: &warn,
        terminal: Some(&wiring.terminal),
        login: &mut login,
        probe: &*session.client,
        signals: &signals,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    run_up(&mut ports)?;
    Ok(0)
}

pub(crate) fn configure(
    session: &mut Session<'_>,
    command: AgentSetupCommand,
) -> Result<(), Failure> {
    let private_files = PrivateFiles::default();
    let mut prompter = SetupPrompter;
    let clock = crate::agent_setup_profiles::locks::SystemClock;
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
