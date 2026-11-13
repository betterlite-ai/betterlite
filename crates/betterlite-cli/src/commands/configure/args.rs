use std::convert::Infallible;

use clap::builder::ValueParser;
use clap::{Args, Subcommand};
use secrecy::SecretString;

use crate::agent_setup::MODEL_OPTION_HELP;

pub const CONFIGURE_ABOUT: &str =
    "Apply saved setup, or choose agents and models on the first run.";
pub const RECONFIGURE_ABOUT: &str =
    "Edit saved gateway, key and model choices, using current choices as defaults.";
pub const UNCONFIGURE_ABOUT: &str =
    "Disconnect agents while retaining saved setup for `lite configure`.";
pub const CONFIGURE_CLAUDE_ABOUT: &str =
    "Apply Claude Code's saved setup, or save the supplied settings.";
pub const CONFIGURE_CODEX_ABOUT: &str = "Apply Codex's saved setup, or save the supplied settings.";
pub const RECONFIGURE_CLAUDE_ABOUT: &str =
    "Edit Claude Code setup, or supply --model / --default-model to apply directly.";
pub const RECONFIGURE_CODEX_ABOUT: &str = "Edit Codex setup, or supply --model to apply directly.";
pub const UNCONFIGURE_CLAUDE_ABOUT: &str =
    "Restore Claude Code settings, including those applied by `lite login --config-claude`.";
pub const UNCONFIGURE_CODEX_ABOUT: &str =
    "Restore Codex settings still holding what configure wrote.";
pub const GROUP_API_KEY_HELP: &str =
    "Long-lived LiteLLM virtual key to save for the selected agents.";
pub const GROUP_GATEWAY_URL_HELP: &str = "Gateway URL, including any deployment path prefix.";
pub const TARGET_API_KEY_HELP: &str = "Long-lived LiteLLM virtual key, or reuse the saved key.";
pub const TARGET_GATEWAY_URL_HELP: &str = "Gateway URL, or reuse the saved gateway.";
pub const DEFAULT_MODEL_HELP: &str =
    "Stop pinning a starting model; let Claude Code choose its default.";
pub const CONFIGURE_CODEX_MODEL_HELP: &str =
    "Gateway model to start on; required only for first-time setup.";
pub const RECONFIGURE_CODEX_MODEL_HELP: &str =
    "Gateway model to start on; omit to open the setup wizard.";
pub const UNCONFIGURE_FORGET_HELP: &str = "Also delete saved setups and their keys.";
pub const UNCONFIGURE_CLAUDE_FORGET_HELP: &str = "Also delete the saved Claude Code setup and key.";
pub const UNCONFIGURE_CODEX_FORGET_HELP: &str = "Also delete the saved Codex setup and key.";

#[derive(Args)]
pub struct GroupConnection {
    #[arg(
        long,
        value_name = "TEXT",
        help = GROUP_API_KEY_HELP,
        value_parser = ValueParser::new(|value: &str| -> Result<SecretString, Infallible> {
            Ok(SecretString::from(value))
        })
    )]
    pub api_key: Option<SecretString>,
    #[arg(
        long = "gateway-url",
        visible_alias = "base-url",
        value_name = "TEXT",
        help = GROUP_GATEWAY_URL_HELP
    )]
    pub gateway_url: Option<String>,
}

#[derive(Args)]
pub struct ConnectionFlags {
    #[arg(
        long,
        value_name = "TEXT",
        help = TARGET_API_KEY_HELP,
        value_parser = ValueParser::new(|value: &str| -> Result<SecretString, Infallible> {
            Ok(SecretString::from(value))
        })
    )]
    pub api_key: Option<SecretString>,
    #[arg(
        long = "gateway-url",
        visible_alias = "base-url",
        value_name = "TEXT",
        help = TARGET_GATEWAY_URL_HELP
    )]
    pub gateway_url: Option<String>,
}

#[derive(Args)]
#[command(about = CONFIGURE_ABOUT)]
pub struct ConfigureArgs {
    #[command(flatten)]
    pub connection: GroupConnection,
    #[command(subcommand)]
    pub command: Option<ConfigureTarget>,
}

#[derive(Subcommand)]
pub enum ConfigureTarget {
    #[command(about = CONFIGURE_CLAUDE_ABOUT)]
    Claude {
        #[command(flatten)]
        connection: ConnectionFlags,
        #[arg(long, value_name = "TEXT", help = MODEL_OPTION_HELP)]
        model: Option<String>,
        #[arg(long = "default-model", help = DEFAULT_MODEL_HELP)]
        default_model: bool,
    },
    #[command(about = CONFIGURE_CODEX_ABOUT)]
    Codex {
        #[command(flatten)]
        connection: ConnectionFlags,
        #[arg(long, value_name = "TEXT", help = CONFIGURE_CODEX_MODEL_HELP)]
        model: Option<String>,
    },
}

#[derive(Args)]
#[command(about = RECONFIGURE_ABOUT)]
pub struct ReconfigureArgs {
    #[command(flatten)]
    pub connection: GroupConnection,
    #[command(subcommand)]
    pub command: Option<ReconfigureTarget>,
}

#[derive(Subcommand)]
pub enum ReconfigureTarget {
    #[command(about = RECONFIGURE_CLAUDE_ABOUT)]
    Claude {
        #[command(flatten)]
        connection: ConnectionFlags,
        #[arg(long, value_name = "TEXT", help = MODEL_OPTION_HELP)]
        model: Option<String>,
        #[arg(long = "default-model", help = DEFAULT_MODEL_HELP)]
        default_model: bool,
    },
    #[command(about = RECONFIGURE_CODEX_ABOUT)]
    Codex {
        #[command(flatten)]
        connection: ConnectionFlags,
        #[arg(long, value_name = "TEXT", help = RECONFIGURE_CODEX_MODEL_HELP)]
        model: Option<String>,
    },
}

impl ReconfigureArgs {
    pub(crate) fn into_configure(self) -> ConfigureArgs {
        ConfigureArgs {
            connection: self.connection,
            command: self.command.map(|command| match command {
                ReconfigureTarget::Claude {
                    connection,
                    model,
                    default_model,
                } => ConfigureTarget::Claude {
                    connection,
                    model,
                    default_model,
                },
                ReconfigureTarget::Codex { connection, model } => {
                    ConfigureTarget::Codex { connection, model }
                }
            }),
        }
    }
}

#[derive(Args)]
#[command(about = UNCONFIGURE_ABOUT)]
pub struct UnconfigureArgs {
    #[arg(long, help = UNCONFIGURE_FORGET_HELP)]
    pub forget: bool,
    #[command(subcommand)]
    pub command: Option<UnconfigureTarget>,
}

#[derive(Subcommand)]
pub enum UnconfigureTarget {
    #[command(about = UNCONFIGURE_CLAUDE_ABOUT)]
    Claude {
        #[arg(long, help = UNCONFIGURE_CLAUDE_FORGET_HELP)]
        forget: bool,
    },
    #[command(about = UNCONFIGURE_CODEX_ABOUT)]
    Codex {
        #[arg(long, help = UNCONFIGURE_CODEX_FORGET_HELP)]
        forget: bool,
    },
}
