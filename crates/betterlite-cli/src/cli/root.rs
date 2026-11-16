use betterlite_config::loader::EnvironmentLookup;
use clap::{Arg, ArgAction, Command};

use crate::agent_dispatch::agent_commands;
use crate::config::hidden_command_names;

mod management;
mod session;
mod tools;

pub(crate) const SHOW_VERSION_ARG: &str = "show_version";
pub(crate) const BASE_URL_ARG: &str = "base_url";
pub(crate) const API_KEY_ARG: &str = "api_key";

const ABOUT: &str = "betterlite Proxy CLI - Manage your betterlite proxy server";
const VERSION_HELP: &str = "Show the betterlite proxy CLI and server version and exit.";
const BASE_URL_HELP: &str = "Base URL of the betterlite proxy server [env: LITELLM_PROXY_URL=]";
const API_KEY_HELP: &str = "API key for authentication [env: LITELLM_PROXY_API_KEY=]";

#[must_use]
pub(crate) fn root_command(environment: &dyn EnvironmentLookup) -> Command {
    let hidden = hidden_command_names(environment);
    let mut subcommands = family_commands();
    subcommands.extend(agent_commands());
    Command::new("lite")
        .about(ABOUT)
        .disable_help_subcommand(true)
        .disable_version_flag(true)
        .arg(
            Arg::new(SHOW_VERSION_ARG)
                .short('v')
                .long("version")
                .action(ArgAction::SetTrue)
                .help(VERSION_HELP),
        )
        .arg(
            Arg::new(BASE_URL_ARG)
                .long("base-url")
                .value_name("TEXT")
                .help(BASE_URL_HELP),
        )
        .arg(
            Arg::new(API_KEY_ARG)
                .long("api-key")
                .value_name("TEXT")
                .help(API_KEY_HELP),
        )
        .subcommands(subcommands)
        .mut_subcommands(move |subcommand| {
            if hidden.contains(subcommand.get_name()) {
                subcommand.hide(true)
            } else {
                subcommand
            }
        })
}

fn family_commands() -> Vec<Command> {
    let mut commands = session::commands();
    commands.extend(management::commands());
    commands.extend(tools::commands());
    commands
}
