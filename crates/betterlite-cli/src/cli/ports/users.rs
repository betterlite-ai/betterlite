use crate::commands::users::{UsersCommand, run_users_command};

use super::super::Failure;
use super::super::session::Session;

pub(crate) fn run(session: &mut Session<'_>, command: UsersCommand) -> Result<(), Failure> {
    Ok(run_users_command(
        command,
        &session.client,
        &mut session.streams.stdout,
    )?)
}
