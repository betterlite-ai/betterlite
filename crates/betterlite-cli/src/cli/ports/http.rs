use crate::commands::http::{HttpCommand, run_http_command};

use super::super::Failure;
use super::super::session::Session;

pub(crate) fn run(session: &mut Session<'_>, command: HttpCommand) -> Result<(), Failure> {
    Ok(run_http_command(
        command,
        &session.context,
        &session.client,
        &mut session.streams.stdout,
        &mut session.streams.stderr,
    )?)
}
