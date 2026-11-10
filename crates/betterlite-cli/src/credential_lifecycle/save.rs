use secrecy::ExposeSecret as _;

use crate::keyring::{Read, SecretVault, SecretWrite, Write};
use crate::token_records::record::{CliTokenRecord, holds_secret, without_secret};
use crate::token_records::secret::{CliTokenSecret, decode_secret, encode_secret};

use super::files::CredentialFiles;
use super::outcome::{CredentialNotRecorded, CredentialNotSaved, SecretSave, saved_as};

#[must_use]
pub async fn save_cli_token(
    files: &dyn CredentialFiles,
    vault: &mut (dyn SecretVault + Send + Sync),
    record: CliTokenRecord,
) -> SecretSave {
    let stamped = stamp_past_every_stored_login(files, &*vault, record);
    let secret = if holds_secret(&stamped) {
        match encode_secret(&stamped) {
            Ok(blob) => Some(blob),
            Err(source) => return SecretSave::CredentialNotSaved(CredentialNotSaved { source }),
        }
    } else {
        None
    };
    let staged = match files.stage(&without_secret(&stamped)).await {
        Ok(staged) => staged,
        Err(source) => return SecretSave::CredentialNotSaved(CredentialNotSaved { source }),
    };
    let outcome = secret.map_or(Ok(Write::SecretStored), |blob| vault.write(&blob));
    if matches!(outcome, Ok(Write::SecretStored)) {
        if files.commit(&staged).await {
            return saved_as(&outcome);
        }
        return SecretSave::CredentialNotRecorded(CredentialNotRecorded);
    }
    files.discard(&staged).await;
    keep_the_secret_in_the_file(files, &stamped, outcome).await
}

async fn keep_the_secret_in_the_file(
    files: &dyn CredentialFiles,
    record: &CliTokenRecord,
    outcome: SecretWrite,
) -> SecretSave {
    match files.write(record).await {
        Ok(()) => saved_as(&outcome),
        Err(source) => SecretSave::CredentialNotSaved(CredentialNotSaved { source }),
    }
}

fn stamp_past_every_stored_login(
    files: &dyn CredentialFiles,
    vault: &(dyn SecretVault + Send + Sync),
    mut record: CliTokenRecord,
) -> CliTokenRecord {
    let highest = highest_stamp_already_stored(files, vault, &record.base_url);
    if highest < record.timestamp {
        return record;
    }
    record.timestamp = highest.next_up();
    record
}

fn highest_stamp_already_stored(
    files: &dyn CredentialFiles,
    vault: &(dyn SecretVault + Send + Sync),
    base_url: &str,
) -> f64 {
    let previous = files
        .read()
        .map_or(f64::NEG_INFINITY, |record| record.timestamp);
    let secret =
        stored_secret(vault, base_url).map_or(f64::NEG_INFINITY, |secret| secret.timestamp);
    f64::max(previous, secret)
}

fn stored_secret(
    vault: &(dyn SecretVault + Send + Sync),
    base_url: &str,
) -> Option<CliTokenSecret> {
    match vault.read() {
        Ok(Read::SecretFound(blob)) => decode_secret(blob.expose_secret(), base_url),
        Ok(Read::SecretMissing) | Err(_) => None,
    }
}
