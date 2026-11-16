use crate::config::commands::{ConfigCommand, run_config_command};

use super::super::Failure;
use super::super::session::Session;

pub(crate) fn run(session: &mut Session<'_>, command: ConfigCommand) -> Result<(), Failure> {
    Ok(run_config_command(
        command,
        session.environment,
        &mut session.streams.stdout,
        &mut session.streams.stderr,
    )?)
}
