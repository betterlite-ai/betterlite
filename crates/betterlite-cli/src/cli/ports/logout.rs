use std::sync::Arc;

use crate::session::logout::{LogoutRequest, logout as run_logout};

use super::super::Failure;
use super::super::session::Session;
use super::wiring::token_file;

pub(crate) fn run(session: &mut Session<'_>) -> Result<(), Failure> {
    let files = token_file(session)?;
    let mut stdout = session.streams.stdout.clone();
    Ok(run_logout(&mut LogoutRequest {
        files: &files,
        vault: &mut *session.vault,
        http: Arc::clone(&session.client),
        token_path: &files.path,
        stdout: &mut stdout,
    })?)
}
