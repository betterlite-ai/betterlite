use clap::ArgMatches;
use clap::FromArgMatches as _;

use crate::login::LoginArgs;
use crate::session::auth_command::AuthCommand;

use super::super::Failure;
use super::super::session::Session;
use super::super::{flows, ports};

pub(crate) fn run(session: &mut Session<'_>, name: &str, sub: &ArgMatches) -> Result<u8, Failure> {
    let run = match name {
        "login" => flows::login(session, LoginArgs::from_arg_matches(sub)?),
        "logout" => ports::logout::run(session),
        "whoami" => ports::whoami::run(session),
        "auth" => ports::auth::run(session, AuthCommand::from_arg_matches(sub)?),
        #[cfg(unix)]
        "up" => flows::up(session).map(|_code| ()),
        #[cfg(not(unix))]
        "up" => Err(Failure::Plain(String::from("`lite up` needs Unix signals"))),
        "down" => ports::down::run(session),
        _unknown => return Ok(0),
    };
    run.map(|()| 0)
}
