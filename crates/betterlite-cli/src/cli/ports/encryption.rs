use crate::commands::encryption::{EncryptionCommand, EncryptionPorts, run_encryption_command};

use super::super::Failure;
use super::super::session::Session;

pub(crate) fn run(session: &mut Session<'_>, command: EncryptionCommand) -> Result<(), Failure> {
    let ports = EncryptionPorts {
        context: &session.context,
        client: &session.client,
        redactor: &session.redactor,
        stdout: &mut session.streams.stdout,
        stderr: &mut session.streams.stderr,
    };
    Ok(run_encryption_command(command, ports)?)
}
