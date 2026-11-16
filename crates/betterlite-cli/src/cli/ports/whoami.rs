use std::sync::Arc;

use crate::session::whoami::{WhoamiRequest, whoami as run_whoami};

use super::super::Failure;
use super::super::session::Session;
use super::wiring::token_file;

pub(crate) fn run(session: &mut Session<'_>) -> Result<(), Failure> {
    let files = token_file(session)?;
    let mut stdout = session.streams.stdout.clone();
    Ok(run_whoami(&mut WhoamiRequest {
        files: &files,
        vault: &mut *session.vault,
        clock: Arc::clone(&session.clock),
        expiration_hours: session.expiration_hours,
        stdout: &mut stdout,
    })?)
}
