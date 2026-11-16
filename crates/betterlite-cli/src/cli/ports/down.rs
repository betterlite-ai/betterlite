use betterlite_logging::private_json::files::PrivateFiles;

use crate::settings_backup::down::down as restore_settings;

use super::super::Failure;
use super::super::session::Session;

pub(crate) fn run(session: &mut Session<'_>) -> Result<(), Failure> {
    let files = PrivateFiles::default();
    Ok(restore_settings(
        session.environment,
        &files,
        &mut session.streams.stdout,
    )?)
}
