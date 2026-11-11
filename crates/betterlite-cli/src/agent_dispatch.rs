pub mod error;
pub mod key;
pub mod launch;

use std::iter;
use std::path::Path;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::files::PrivateFiles;
use clap::{Arg, ArgAction, Command};

use crate::agent_launch::{
    EnvVars, KNOWN_AGENTS, KeyProbe, Platform, ProcessLauncher, Terminal, agent_profile,
};
use crate::agent_run::messages::Messages;
use crate::agent_run::prepare::Preparers;
use crate::agent_run::resolver::BinaryResolver;
use crate::agent_run::sync::ModelSync;
use crate::cli_context::CliContext;
use crate::credential_lifecycle::files::CredentialFiles;
use crate::credentials::notice::MessageSink;
use crate::keyring::SecretVault;
use crate::login::ports::LoginPorts;
use crate::login::ports::{FlowPorts, TerminalWriter};
use crate::login::{Interrupts, LoginArgs, run_login};
use crate::pkce_login::{Clock, Entropy, Http, WallClock};
use crate::sso_polling::polling::Sleeper;
use crate::sso_polling::terminal::SelectionInput;
use crate::sso_polling::transport::PollTransport;

use error::AgentDispatchError;
use key::resolve_api_key;
use launch::launch_agent;

pub const SKIP_VERIFY_HELP: &str = "Skip the pre-launch key check against the proxy.";
pub const SKIP_VERIFY: &str = "skip-verify";
const SKIP_VERIFY_FLAG: &str = "--skip-verify";
const TERMINATOR: &str = "--";
const FORWARDED_ARGS: &str = "args";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentCommandArgs {
    pub skip_verify: bool,
    pub forwarded: Vec<String>,
}

#[must_use]
pub fn agent_commands() -> Vec<Command> {
    KNOWN_AGENTS
        .iter()
        .map(|(binary, display_name, _profiles, hidden)| {
            build_command(binary, display_name, *hidden)
        })
        .collect()
}

fn build_command(binary: &'static str, display_name: &str, hidden: bool) -> Command {
    let short_help = format!("Run {display_name} through your LiteLLM proxy");
    let long_help = format!(
        "Run {display_name} routed through your LiteLLM proxy.\n\nLogs in with LiteLLM if needed, verifies your key against the proxy, exports the env vars {binary} reads, then hands off. Any arguments are forwarded to `{binary}`."
    );
    Command::new(binary)
        .about(short_help)
        .long_about(long_help)
        .hide(hidden)
        .arg(
            Arg::new(SKIP_VERIFY)
                .long(SKIP_VERIFY)
                .action(ArgAction::SetTrue)
                .help(SKIP_VERIFY_HELP),
        )
        .arg(
            Arg::new(FORWARDED_ARGS)
                .value_name("ARGS")
                .num_args(0..)
                .allow_hyphen_values(true),
        )
}

#[must_use]
pub fn parse_agent_args(tail: &[String]) -> AgentCommandArgs {
    let mut skip_verify = false;
    let mut forwarded = Vec::new();
    let mut terminated = false;
    for token in tail {
        if terminated {
            forwarded.push(token.clone());
        } else if token.as_str() == TERMINATOR {
            terminated = true;
        } else if token.as_str() == SKIP_VERIFY_FLAG {
            skip_verify = true;
        } else {
            forwarded.push(token.clone());
        }
    }
    AgentCommandArgs {
        skip_verify,
        forwarded,
    }
}

pub struct AgentPorts<'agent, H, E, W> {
    pub context: &'agent CliContext,
    pub environment: &'agent (dyn EnvironmentLookup + Send + Sync),
    pub files: &'agent dyn CredentialFiles,
    pub vault: &'agent mut (dyn SecretVault + Send + Sync),
    pub private_files: &'agent PrivateFiles,
    pub http: Arc<H>,
    pub clock: Arc<W>,
    pub entropy: &'agent mut E,
    pub sleeper: &'agent dyn Sleeper,
    pub selection: &'agent mut dyn SelectionInput,
    pub browser: MessageSink<'agent>,
    pub interrupt: &'agent dyn Interrupts,
    pub token_path: &'agent Path,
    pub binary_path: Option<&'agent Path>,
    pub base_env: &'agent EnvVars,
    pub home: Option<&'agent Path>,
    pub resolver: &'agent dyn BinaryResolver,
    pub probe: &'agent dyn KeyProbe,
    pub sync: &'agent dyn ModelSync,
    pub preparers: &'agent Preparers<'agent>,
    pub launcher: &'agent dyn ProcessLauncher,
    pub platform: Platform,
    pub terminal: Option<&'agent dyn Terminal>,
    pub messages: &'agent mut dyn Messages,
    pub stdout: &'agent mut TerminalWriter,
}

pub fn run_agent_command<H, E, W>(
    binary: &str,
    args: AgentCommandArgs,
    ports: &mut AgentPorts<'_, H, E, W>,
) -> Result<i32, AgentDispatchError>
where
    H: Http + PollTransport + Send + Sync + 'static,
    E: Entropy,
    W: Clock + WallClock + Send + Sync + 'static,
{
    let started_interactive = ports.terminal.is_some_and(Terminal::is_interactive);
    let api_key = resolve_api_key(ports)?;
    let (display_name, _profiles) = agent_profile(binary);
    writeln!(
        ports.stdout,
        "litellm: routing {display_name} through proxy at {}",
        ports.context.base_url.trim_end_matches('/')
    )?;
    let command: Vec<String> = iter::once(binary.to_owned())
        .chain(args.forwarded)
        .collect();
    let terminal = if started_interactive {
        ports.terminal
    } else {
        None
    };
    launch_agent(&api_key, &command, args.skip_verify, terminal, ports)
}

pub fn login_through<H, E, W>(ports: &mut AgentPorts<'_, H, E, W>) -> Result<(), AgentDispatchError>
where
    H: Http + PollTransport + Send + Sync + 'static,
    E: Entropy,
    W: Clock + WallClock + Send + Sync + 'static,
{
    let clock = Arc::clone(&ports.clock);
    let flow = FlowPorts {
        context: ports.context,
        environment: ports.environment,
        files: ports.files,
        vault: &mut *ports.vault,
        private_files: ports.private_files,
        http: Arc::clone(&ports.http),
        clock: &*clock,
        wall_clock: &*clock,
        entropy: &mut *ports.entropy,
        sleeper: ports.sleeper,
        selection: &mut *ports.selection,
        browser: ports.browser,
        interrupt: ports.interrupt,
        token_path: ports.token_path,
        binary_path: ports.binary_path,
    };
    let mut login = LoginPorts {
        flow,
        stdout: &mut *ports.stdout,
    };
    run_login(LoginArgs::default(), &mut login).map_err(AgentDispatchError::Login)
}

mod tests;
