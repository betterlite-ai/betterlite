use crate::token_records::record::{CliTokenRecord, holds_secret, without_secret};

use super::files::CredentialFiles;

pub(super) async fn scrub_file_secret(
    files: &dyn CredentialFiles,
    record: &CliTokenRecord,
) -> bool {
    if !holds_secret(record) {
        return true;
    }
    if let Ok(staged) = files.stage(&without_secret(record)).await
        && files.commit(&staged).await
    {
        return true;
    }
    files.overwrite_scrubbed(record).await
}
