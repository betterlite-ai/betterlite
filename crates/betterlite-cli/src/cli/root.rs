use betterlite_config::loader::EnvironmentLookup;
use clap::{Arg, ArgAction, Args as _, Command, Subcommand as _};

use crate::agent_dispatch::agent_commands;
use crate::autoroute::commands::args::{AUTOROUTE_ABOUT, AutorouteArgs};
use crate::commands::chat::ChatArgs;
use crate::commands::configure::args::{
    CONFIGURE_ABOUT, ConfigureArgs, RECONFIGURE_ABOUT, ReconfigureArgs, UNCONFIGURE_ABOUT,
    UnconfigureArgs,
};
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
use crate::config::hidden_command_names;
use crate::login::{LOGIN_ABOUT, LoginArgs};
use crate::session::auth_command::{AUTH_ABOUT, AuthCommand};
use crate::session::logout::LOGOUT_ABOUT;
use crate::session::whoami::WHOAMI_ABOUT;
use crate::settings_backup::down::{DOWN_ABOUT, DOWN_LONG_ABOUT};
use crate::statusline::StatuslineArgs;
use crate::up::UP_ABOUT;

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
    vec![
        LoginArgs::augment_args(Command::new("login").about(LOGIN_ABOUT)),
        Command::new("logout").about(LOGOUT_ABOUT),
        Command::new("whoami").about(WHOAMI_ABOUT),
        AuthCommand::augment_subcommands(
            Command::new("auth")
                .about(AUTH_ABOUT)
                .disable_help_subcommand(true),
        ),
        ModelsCommand::augment_subcommands(Command::new("models"))
            .about("Manage models on your betterlite proxy server"),
        CredentialsCommand::augment_subcommands(Command::new("credentials"))
            .about("Manage credentials for the betterlite proxy server"),
        UsersCommand::augment_subcommands(Command::new("users"))
            .about("Manage users on your betterlite proxy server"),
        TeamsCommand::augment_subcommands(Command::new("teams")),
        ModelGroupsCommand::augment_subcommands(Command::new("model-groups")),
        KeysCommand::augment_subcommands(Command::new("keys")),
        EncryptionCommand::augment_subcommands(
            Command::new("encryption").disable_help_subcommand(true),
        ),
        ConfigCommand::augment_subcommands(
            Command::new("config")
                .about("Manage the stored CLI configuration")
                .disable_help_subcommand(true),
        ),
        ChatArgs::augment_args(Command::new("chat")),
        HttpCommand::augment_subcommands(Command::new("http"))
            .about("Make HTTP requests to the proxy server"),
        DebugCommand::augment_subcommands(Command::new("debug")).about(
            "Pull debug reports (spend, request, response, error) for coding-agent sessions",
        ),
        Command::new("up").about(UP_ABOUT),
        Command::new("down")
            .about(DOWN_ABOUT)
            .long_about(DOWN_LONG_ABOUT),
        AutorouteArgs::augment_args(
            Command::new("autoroute")
                .about(AUTOROUTE_ABOUT)
                .disable_help_subcommand(true),
        ),
        ConfigureArgs::augment_args(
            Command::new("configure")
                .about(CONFIGURE_ABOUT)
                .disable_help_subcommand(true),
        ),
        ReconfigureArgs::augment_args(
            Command::new("reconfigure")
                .about(RECONFIGURE_ABOUT)
                .disable_help_subcommand(true),
        ),
        UnconfigureArgs::augment_args(
            Command::new("unconfigure")
                .about(UNCONFIGURE_ABOUT)
                .disable_help_subcommand(true),
        ),
        StatuslineArgs::augment_args(Command::new("statusline").hide(true)),
        Command::new("version").about("Show version information"),
    ]
}
