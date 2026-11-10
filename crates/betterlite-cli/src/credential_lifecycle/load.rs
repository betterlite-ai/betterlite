use secrecy::ExposeSecret as _;

use crate::keyring::{Read, SecretVault, Write};
use crate::token_records::record::{CliTokenRecord, holds_secret, without_secret};
use crate::token_records::secret::{CliTokenSecret, decode_secret, encode_secret};

use super::files::CredentialFiles;
use super::scrub::scrub_file_secret;

#[must_use]
pub async fn load_cli_token(
    files: &dyn CredentialFiles,
    vault: &mut (dyn SecretVault + Send + Sync),
) -> Option<CliTokenRecord> {
    let stored = files.read();
    if let Some(record) = stored {
        return resolve_secret(files, vault, record).await;
    }
    None
}

#[must_use]
pub(super) async fn resolve_secret(
    files: &dyn CredentialFiles,
    vault: &mut (dyn SecretVault + Send + Sync),
    record: CliTokenRecord,
) -> Option<CliTokenRecord> {
    match vault.read() {
        Ok(Read::SecretFound(blob)) => {
            apply_vault_secret(files, vault, record, blob.expose_secret()).await
        }
        Ok(Read::SecretMissing) => migrate_file_secret(files, vault, record, false).await,
        Err(_unusable) => Some(record),
    }
}

async fn apply_vault_secret(
    files: &dyn CredentialFiles,
    vault: &mut (dyn SecretVault + Send + Sync),
    record: CliTokenRecord,
    blob: &str,
) -> Option<CliTokenRecord> {
    let Some(secret) = decode_secret(blob, &record.base_url) else {
        return migrate_file_secret(files, vault, record, false).await;
    };
    if holds_secret(&record) && secret.timestamp <= record.timestamp {
        let rejoined = rejoin(&record, &secret);
        return migrate_file_secret(files, vault, rejoined, true).await;
    }
    let _unused = scrub_file_secret(files, &record).await;
    Some(with_secret(record, &secret))
}

fn with_secret(mut record: CliTokenRecord, secret: &CliTokenSecret) -> CliTokenRecord {
    record.key.clone_from(&secret.key);
    record.jwt_token.clone_from(&secret.jwt_token);
    record.refresh_token.clone_from(&secret.refresh_token);
    record.timestamp = f64::max(secret.timestamp, record.timestamp);
    record
}

fn rejoin(record: &CliTokenRecord, secret: &CliTokenSecret) -> CliTokenRecord {
    if secret.timestamp.to_bits() != record.timestamp.to_bits() {
        return record.clone();
    }
    let mut rejoined = record.clone();
    if rejoined.key.is_none() {
        rejoined.key.clone_from(&secret.key);
    }
    if rejoined.jwt_token.expose_secret().is_empty() {
        rejoined.jwt_token.clone_from(&secret.jwt_token);
    }
    if rejoined.refresh_token.is_none() {
        rejoined.refresh_token.clone_from(&secret.refresh_token);
    }
    rejoined
}

async fn migrate_file_secret(
    files: &dyn CredentialFiles,
    vault: &mut (dyn SecretVault + Send + Sync),
    record: CliTokenRecord,
    replacing: bool,
) -> Option<CliTokenRecord> {
    if !holds_secret(&record) {
        return None;
    }
    let Ok(blob) = encode_secret(&record) else {
        return Some(record);
    };
    let Ok(staged) = files.stage(&without_secret(&record)).await else {
        return Some(record);
    };
    if !matches!(vault.write(&blob), Ok(Write::SecretStored)) {
        files.discard(&staged).await;
        return Some(record);
    }
    let committed = files.commit(&staged).await;
    if !committed {
        let overwritten = files.overwrite_scrubbed(&record).await;
        if !overwritten && !replacing {
            let _unused = vault.erase();
        }
    }
    Some(record)
}
