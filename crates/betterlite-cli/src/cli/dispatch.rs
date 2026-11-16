use clap::ArgMatches;

use crate::agent_dispatch::{AgentCommandArgs, SKIP_VERIFY};
use crate::agent_launch::KNOWN_AGENTS;

use super::Failure;
use super::ports;
use super::session::Session;

mod identity;
mod management;
mod tools;

const FORWARDED_ARGS: &str = "args";

pub(crate) fn run(session: &mut Session<'_>, matches: &ArgMatches) -> Result<u8, Failure> {
    let Some((name, sub)) = matches.subcommand() else {
        return Ok(0);
    };
    match name {
        "login" | "logout" | "whoami" | "auth" | "up" | "down" => identity::run(session, name, sub),
        "models" | "credentials" | "users" | "teams" | "model-groups" | "keys" | "encryption"
        | "config" => management::run(session, name, sub),
        "configure" | "reconfigure" | "unconfigure" | "autoroute" | "chat" | "http" | "debug"
        | "statusline" | "version" => tools::run(session, name, sub),
        agent if is_agent(agent) => agent_command(session, agent, sub),
        // clap rejects any other name before dispatch gets the matches
        _unknown => Ok(0),
    }
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
