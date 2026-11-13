use clap::{Args, Subcommand};

use crate::autoroute::process::DEFAULT_AUTOROUTE_PORT;

pub const AUTOROUTE_ABOUT: &str =
    "QA complexity-based auto-routing against models your key can already use";
pub const CONFIGURE_ABOUT: &str =
    "Discover accessible models and generate an ephemeral auto-router config";
pub const START_ABOUT: &str =
    "Launch the ephemeral auto-router proxy and route Claude Code through it";
pub const STOP_ABOUT: &str =
    "Restore Claude Code settings and stop a leftover ephemeral proxy, if any";
pub const UP_ABOUT: &str = "Deprecated alias of `lite autoroute start`";
pub const DOWN_ABOUT: &str = "Deprecated alias of `lite autoroute stop`";
pub const PORT_HELP: &str =
    "Loopback port for the ephemeral proxy; stable across runs so configured clients keep working.";

#[derive(Args)]
#[command(about = AUTOROUTE_ABOUT)]
pub struct AutorouteArgs {
    #[command(subcommand)]
    pub command: AutorouteCommand,
}

#[derive(Subcommand)]
pub enum AutorouteCommand {
    #[command(about = CONFIGURE_ABOUT)]
    Configure,
    #[command(about = START_ABOUT)]
    Start(StartArgs),
    #[command(about = STOP_ABOUT)]
    Stop,
    #[command(about = UP_ABOUT, hide = true)]
    Up(StartArgs),
    #[command(about = DOWN_ABOUT, hide = true)]
    Down,
}

#[derive(Args)]
pub struct StartArgs {
    #[arg(
        long,
        value_name = "PORT",
        default_value_t = DEFAULT_AUTOROUTE_PORT,
        value_parser = clap::value_parser!(u16).range(1..),
        help = PORT_HELP
    )]
    pub port: u16,
}
