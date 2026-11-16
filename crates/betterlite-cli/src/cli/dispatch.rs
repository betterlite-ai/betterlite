use clap::ArgMatches;
use clap::FromArgMatches as _;

use crate::agent_dispatch::{AgentCommandArgs, SKIP_VERIFY};
use crate::agent_launch::KNOWN_AGENTS;
use crate::autoroute::commands::args::AutorouteArgs;
use crate::commands::chat::ChatArgs;
use crate::commands::configure::AgentSetupCommand;
use crate::commands::configure::args::{ConfigureArgs, ReconfigureArgs, UnconfigureArgs};
use crate::commands::credentials::CredentialsCommand;
use crate::commands::debug::DebugCommand;
use crate::commands::encryption::EncryptionCommand;
use crate::commands::http::HttpCommand;
use crate::commands::keys::KeysCommand;
use crate::commands::model_groups::ModelGroupsCommand;
use crate::commands::models::ModelsCommand;
use crate::commands::teams::TeamsCommand;
use crate::commands::users::UsersCommand;
use crate::config::commands::ConfigCommand;
use crate::login::LoginArgs;
use crate::session::auth_command::AuthCommand;
use crate::statusline::StatuslineArgs;

use super::Failure;
use super::session::Session;
use super::{flows, ports};

const FORWARDED_ARGS: &str = "args";

pub(crate) fn run(session: &mut Session<'_>, matches: &ArgMatches) -> Result<u8, Failure> {
    let Some((name, sub)) = matches.subcommand() else {
        return Ok(0);
    };
    match name {
        "login" | "logout" | "whoami" | "auth" | "up" | "down" => identity(session, name, sub),
        "models" | "credentials" | "users" | "teams" | "model-groups" | "keys" | "encryption"
        | "config" => management(session, name, sub),
        "configure" | "reconfigure" | "unconfigure" | "autoroute" | "chat" | "http" | "debug"
        | "statusline" | "version" => tools(session, name, sub),
        agent if is_agent(agent) => agent_command(session, agent, sub),
        // clap rejects any other name before dispatch gets the matches
        _unknown => Ok(0),
    }
}

fn identity(session: &mut Session<'_>, name: &str, sub: &ArgMatches) -> Result<u8, Failure> {
    let run = match name {
        "login" => flows::login(session, LoginArgs::from_arg_matches(sub)?),
        "logout" => ports::logout(session),
        "whoami" => ports::whoami(session),
        "auth" => ports::auth(session, AuthCommand::from_arg_matches(sub)?),
        "up" => up(session),
        "down" => ports::down(session),
        _unknown => return Ok(0),
    };
    run.map(|()| 0)
}

fn management(session: &mut Session<'_>, name: &str, sub: &ArgMatches) -> Result<u8, Failure> {
    let run = match name {
        "models" => ports::models(session, ModelsCommand::from_arg_matches(sub)?),
        "credentials" => ports::credentials(session, CredentialsCommand::from_arg_matches(sub)?),
        "users" => ports::users(session, UsersCommand::from_arg_matches(sub)?),
        "teams" => ports::teams(session, TeamsCommand::from_arg_matches(sub)?),
        "model-groups" => ports::model_groups(session, ModelGroupsCommand::from_arg_matches(sub)?),
        "keys" => ports::keys(session, KeysCommand::from_arg_matches(sub)?),
        "encryption" => ports::encryption(session, EncryptionCommand::from_arg_matches(sub)?),
        "config" => ports::config(session, ConfigCommand::from_arg_matches(sub)?),
        _unknown => return Ok(0),
    };
    run.map(|()| 0)
}

fn tools(session: &mut Session<'_>, name: &str, sub: &ArgMatches) -> Result<u8, Failure> {
    let run = match name {
        "configure" => flows::configure(
            session,
            AgentSetupCommand::Configure(ConfigureArgs::from_arg_matches(sub)?),
        ),
        "reconfigure" => flows::configure(
            session,
            AgentSetupCommand::Reconfigure(ReconfigureArgs::from_arg_matches(sub)?),
        ),
        "unconfigure" => flows::configure(
            session,
            AgentSetupCommand::Unconfigure(UnconfigureArgs::from_arg_matches(sub)?),
        ),
        "autoroute" => flows::autoroute(session, AutorouteArgs::from_arg_matches(sub)?),
        "chat" => {
            let args = ChatArgs::from_arg_matches(sub)?;
            ports::chat(session, &args)
        }
        "http" => ports::http(session, HttpCommand::from_arg_matches(sub)?),
        "debug" => ports::debug(session, DebugCommand::from_arg_matches(sub)?),
        "statusline" => ports::statusline(session, StatuslineArgs::from_arg_matches(sub)?),
        "version" => ports::version(session),
        _unknown => return Ok(0),
    };
    run.map(|()| 0)
}

fn agent_command(session: &mut Session<'_>, binary: &str, sub: &ArgMatches) -> Result<u8, Failure> {
    ports::agent::run(
        session,
        binary,
        AgentCommandArgs {
            skip_verify: sub.get_flag(SKIP_VERIFY),
            forwarded: forwarded_args(sub),
        },
    )
}

fn forwarded_args(matches: &ArgMatches) -> Vec<String> {
    matches
        .get_many::<String>(FORWARDED_ARGS)
        .map(|values| values.cloned().collect())
        .unwrap_or_default()
}

fn is_agent(name: &str) -> bool {
    KNOWN_AGENTS
        .iter()
        .any(|(binary, _name, _profiles, _hidden)| *binary == name)
}

#[cfg(unix)]
fn up(session: &mut Session<'_>) -> Result<(), Failure> {
    flows::up(session).map(|_code| ())
}

#[cfg(not(unix))]
fn up(_session: &mut Session<'_>) -> Result<(), Failure> {
    Err(Failure::Plain(String::from("`lite up` needs Unix signals")))
}
