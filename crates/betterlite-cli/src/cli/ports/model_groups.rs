use crate::commands::model_groups::{ModelGroupsCommand, run_model_groups_command};

use super::super::Failure;
use super::super::session::Session;

pub(crate) fn run(session: &mut Session<'_>, command: ModelGroupsCommand) -> Result<(), Failure> {
    Ok(run_model_groups_command(
        command,
        &session.client,
        &mut session.streams.stdout,
    )?)
}
