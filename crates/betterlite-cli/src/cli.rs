pub(crate) mod context;
pub(crate) mod dispatch;
pub(crate) mod exit;
pub(crate) mod flows;
pub(crate) mod ports;
pub(crate) mod root;
pub(crate) mod session;
pub(crate) mod streams;
pub(crate) mod transport;
pub(crate) mod version;

pub(crate) mod tests;

use std::env::home_dir;
use std::io::{self, IsTerminal as _, Write as _};
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::sections::chatgpt_token_store::home::HomeEnvironment;

use crate::agent_launch::EnvVars;
use crate::cli_context::CliContext;
use crate::commands::Client;
use crate::keyring::{KeyringVault, SecretVault};
use crate::shell;
use crate::token_records::file::token_file_path;

use self::root::{SHOW_VERSION_ARG, root_command};
use self::session::{CliClock, Session, SessionParts};
use self::streams::Streams;

#[derive(Debug)]
pub enum Failure {
    Plain(String),
    Usage(String),
    Aborted(String),
}

impl Failure {
    #[must_use]
    const fn code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Plain(_) | Self::Aborted(_) => 1,
        }
    }
}

pub struct Cli<'cli> {
    environment: &'cli (dyn EnvironmentLookup + Send + Sync),
    vault: &'cli mut (dyn SecretVault + Send + Sync),
    streams: Streams,
    base_env: EnvVars,
    stdin_is_terminal: bool,
}

#[cfg_attr(test, mutants::skip)]
pub fn run(arguments: &[String]) -> u8 {
    let variables = process_environment();
    let environment = HomeEnvironment {
        variables: variables.clone(),
        home: home_dir(),
    };
    let mut streams = Streams::new(Box::new(io::stdout()), Box::new(io::stderr()));
    let mut vault = match KeyringVault::new(&environment) {
        Ok(vault) => vault,
        Err(error) => {
            let _unused = writeln!(streams.stderr, "Error: {error}");
            return 1;
        }
    };
    let cli = Cli {
        environment: &environment,
        vault: &mut vault,
        streams,
        base_env: variables,
        stdin_is_terminal: io::stdin().is_terminal(),
    };
    run_with(arguments, cli)
}

fn run_with(arguments: &[String], mut cli: Cli<'_>) -> u8 {
    match start(arguments, &mut cli) {
        Ok(code) => code,
        Err(failure) => {
            let _unused = writeln!(cli.streams.stderr, "{}", failure_line(&failure));
            failure.code()
        }
    }
}

fn failure_line(failure: &Failure) -> String {
    match failure {
        Failure::Aborted(_) => String::from("Aborted!"),
        Failure::Plain(message) | Failure::Usage(message) => format!("Error: {message}"),
    }
}

fn start(arguments: &[String], cli: &mut Cli<'_>) -> Result<u8, Failure> {
    let mut argv = Vec::with_capacity(1);
    argv.push(String::from("lite"));
    argv.extend_from_slice(arguments);
    let matches = match root_command(cli.environment).try_get_matches_from(argv) {
        Ok(matches) => matches,
        Err(error) => return Ok(finish_clap(&error, &mut cli.streams)),
    };
    let subcommand = matches.subcommand().map(|(name, _sub)| name.to_owned());
    let token_path = cli.environment.system_home();
    let token_path = token_path.as_deref().map(token_file_path);
    let context = context::resolve_for_run(context::Request {
        environment: cli.environment,
        globals: context::globals(&matches),
        subcommand: subcommand.as_deref(),
        vault: &mut *cli.vault,
        clock: Arc::new(CliClock),
        token_path: token_path.as_deref(),
        stderr: cli.streams.stderr.clone(),
    })?;
    let client = Arc::new(Client::new(&context, cli.environment)?);
    if matches.get_flag(SHOW_VERSION_ARG) {
        version::print_version(&context, &client, &mut cli.streams.stdout)?;
        return Ok(0);
    }
    let mut session = build_session(&context, client, cli)?;
    if subcommand.is_none() {
        let mut reader = shell::StdinReader;
        shell::run(&mut session, &mut reader)?;
        return Ok(0);
    }
    dispatch::run(&mut session, &matches)
}

fn build_session<'run>(
    context: &'run CliContext,
    client: Arc<Client>,
    cli: &'run mut Cli<'_>,
) -> Result<Session<'run>, Failure> {
    Session::new(SessionParts {
        context,
        environment: cli.environment,
        vault: &mut *cli.vault,
        streams: cli.streams.clone(),
        client,
        clock: Arc::new(CliClock),
        base_env: cli.base_env.clone(),
        stdin_is_terminal: cli.stdin_is_terminal,
    })
}

fn finish_clap(error: &clap::Error, streams: &mut Streams) -> u8 {
    let rendered = error.render().to_string();
    let written = if error.use_stderr() {
        write!(streams.stderr, "{rendered}")
    } else {
        write!(streams.stdout, "{rendered}")
    };
    if written.is_err() {
        return 1;
    }
    u8::try_from(error.exit_code()).unwrap_or(2)
}
