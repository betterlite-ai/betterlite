use std::sync::Arc;

use crate::session::auth_command::{AuthCommand, run_auth_command};
use crate::session::print_token::PrintTokenRequest;

use super::super::Failure;
use super::super::session::Session;
use super::wiring::token_file;

pub(crate) fn run(session: &mut Session<'_>, command: AuthCommand) -> Result<(), Failure> {
    let files = token_file(session)?;
    let mut stdout = session.streams.stdout.clone();
    let mut stderr = session.streams.stderr.clone();
    Ok(run_auth_command(
        command,
        &mut PrintTokenRequest {
            context: &session.context,
            files: &files,
            vault: &mut *session.vault,
            http: Arc::clone(&session.client),
            clock: Arc::clone(&session.clock),
            expiration_hours: session.expiration_hours,
            token_path: &files.path,
            stdout: &mut stdout,
            stderr: &mut stderr,
        },
    )?)
}
