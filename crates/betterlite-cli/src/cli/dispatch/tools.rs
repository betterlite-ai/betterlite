use clap::ArgMatches;
use clap::FromArgMatches as _;

use crate::autoroute::commands::args::AutorouteArgs;
use crate::commands::chat::ChatArgs;
use crate::commands::configure::AgentSetupCommand;
use crate::commands::configure::args::{ConfigureArgs, ReconfigureArgs, UnconfigureArgs};
use crate::commands::debug::DebugCommand;
use crate::commands::http::HttpCommand;
use crate::statusline::StatuslineArgs;

use super::super::Failure;
use super::super::session::Session;
use super::super::{flows, ports};

pub(crate) fn run(session: &mut Session<'_>, name: &str, sub: &ArgMatches) -> Result<u8, Failure> {
    let run = match name {
        "configure" => flows::tools::configure(
            session,
            AgentSetupCommand::Configure(ConfigureArgs::from_arg_matches(sub)?),
        ),
        "reconfigure" => flows::tools::configure(
            session,
            AgentSetupCommand::Reconfigure(ReconfigureArgs::from_arg_matches(sub)?),
        ),
        "unconfigure" => flows::tools::configure(
            session,
            AgentSetupCommand::Unconfigure(UnconfigureArgs::from_arg_matches(sub)?),
        ),
        "autoroute" => flows::tools::autoroute(session, AutorouteArgs::from_arg_matches(sub)?),
        "chat" => {
            let args = ChatArgs::from_arg_matches(sub)?;
            ports::chat::run(session, &args)
        }
        "http" => ports::http::run(session, HttpCommand::from_arg_matches(sub)?),
        "debug" => ports::debug::run(session, DebugCommand::from_arg_matches(sub)?),
        "statusline" => ports::statusline::run(session, StatuslineArgs::from_arg_matches(sub)?),
        "version" => ports::version::run(session),
        _unknown => return Ok(0),
    };
    run.map(|()| 0)
}
