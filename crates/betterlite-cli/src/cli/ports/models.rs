use crate::commands::models::{ModelsCommand, run_models_command};

use super::super::Failure;
use super::super::session::Session;

pub(crate) fn run(session: &mut Session<'_>, command: ModelsCommand) -> Result<(), Failure> {
    Ok(run_models_command(
        command,
        &session.client,
        &mut session.streams.stdout,
        &mut session.streams.stderr,
    )?)
}
