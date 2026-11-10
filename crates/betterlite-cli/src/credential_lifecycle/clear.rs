use crate::keyring::{Erase, SecretErase, SecretVault};
use crate::token_records::record::{CliTokenRecord, without_secret};

use super::files::CredentialFiles;
use super::outcome::{CredentialNotCleared, SecretClear, cleared_as};
use super::scrub::scrub_file_secret;

#[must_use]
pub async fn clear_cli_token(
    files: &dyn CredentialFiles,
    vault: &mut (dyn SecretVault + Send + Sync),
) -> SecretClear {
    let outcome = vault.erase();
    let record = files.read();
    let settled = nothing_left_behind(&outcome, record.as_ref());
    if !settled && keep_the_unchecked_note(files, &outcome, record.as_ref()).await {
        return cleared_as(&outcome);
    }
    let removal = files.remove().await;
    match (removal, record.as_ref()) {
        (Err(not_cleared), Some(existing)) => {
            if !scrub_file_secret(files, existing).await {
                return SecretClear::CredentialNotCleared(CredentialNotCleared {
                    source: not_cleared,
                });
            }
        }
        (Ok(()), Some(existing)) if outcome.is_err() => {
            rewrite_the_note(files, existing).await;
        }
        (Ok(()), _) | (Err(_), None) => {}
    }
    if settled {
        SecretClear::SecretErased
    } else {
        cleared_as(&outcome)
    }
}

const fn nothing_left_behind(outcome: &SecretErase, record: Option<&CliTokenRecord>) -> bool {
    match outcome {
        Ok(Erase::SecretErased) => true,
        Ok(Erase::SecretStranded) => false,
        Err(_unusable) => record.is_none(),
    }
}

async fn keep_the_unchecked_note(
    files: &dyn CredentialFiles,
    outcome: &SecretErase,
    record: Option<&CliTokenRecord>,
) -> bool {
    let Some(existing) = record else {
        return false;
    };
    if !outcome.is_err() {
        return false;
    }
    scrub_file_secret(files, existing).await
}

async fn rewrite_the_note(files: &dyn CredentialFiles, record: &CliTokenRecord) {
    if let Ok(staged) = files.stage(&without_secret(record)).await {
        let _unused = files.commit(&staged).await;
    }
}
