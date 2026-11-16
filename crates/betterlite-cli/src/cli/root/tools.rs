use clap::{Args as _, Command};

use crate::autoroute::commands::args::{AUTOROUTE_ABOUT, AutorouteArgs};
use crate::commands::configure::args::{
    CONFIGURE_ABOUT, ConfigureArgs, RECONFIGURE_ABOUT, ReconfigureArgs, UNCONFIGURE_ABOUT,
    UnconfigureArgs,
};
use crate::statusline::StatuslineArgs;

pub(crate) fn commands() -> Vec<Command> {
    vec![
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
