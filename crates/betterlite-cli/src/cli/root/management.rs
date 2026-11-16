use clap::{Args as _, Command, Subcommand as _};

use crate::commands::chat::ChatArgs;
use crate::commands::debug::DebugCommand;
use crate::commands::http::HttpCommand;
use crate::config::commands::ConfigCommand;
use crate::settings_backup::down::{DOWN_ABOUT, DOWN_LONG_ABOUT};
use crate::up::UP_ABOUT;

pub(crate) fn commands() -> Vec<Command> {
    vec![
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
    ]
}
