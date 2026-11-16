use std::env::temp_dir;
use std::io;

use crate::statusline::{StatuslineArgs, StatuslinePorts, run as run_statusline};

use super::super::Failure;
use super::super::session::Session;

pub(crate) fn run(session: &mut Session<'_>, args: StatuslineArgs) -> Result<(), Failure> {
    let home = session
        .home
        .clone()
        .ok_or_else(|| Failure::Plain(String::from("cannot find your home directory")))?;
    let temp = temp_dir();
    let mut stdin = io::stdin();
    let mut ports = StatuslinePorts {
        stdin: &mut stdin,
        stdout: &mut session.streams.stdout,
        environment: &session.base_env,
        home_dir: &home,
        temp_dir: &temp,
    };
    Ok(run_statusline(args, &mut ports)?)
}
