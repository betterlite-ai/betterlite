use crate::commands::credentials::{CredentialsCommand, run_credentials_command};

use super::super::Failure;
use super::super::session::Session;

pub(crate) fn run(session: &mut Session<'_>, command: CredentialsCommand) -> Result<(), Failure> {
    Ok(run_credentials_command(
        command,
        &session.client,
        &mut session.streams.stdout,
        &mut session.streams.stderr,
    )?)
}
