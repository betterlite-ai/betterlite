use crate::commands::keys::{KeysCommand, KeysPorts, run_keys_command};

use super::super::Failure;
use super::super::session::Session;

pub(crate) fn run(session: &mut Session<'_>, command: KeysCommand) -> Result<(), Failure> {
    let ports = KeysPorts {
        context: &session.context,
        client: &session.client,
        redactor: &session.redactor,
        stdout: &mut session.streams.stdout,
        stderr: &mut session.streams.stderr,
    };
    Ok(run_keys_command(command, ports)?)
}
