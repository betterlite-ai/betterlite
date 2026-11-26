use std::collections::BTreeMap;

use betterlite_db::encryption_scan::report::{LocationReport, ValueClass};
use betterlite_db::encryption_scan::{
    DbCipher, MigrationJson, ScanError, classify_callback_value, classify_value,
    decode_migration_json, iter_callback_var_dicts, reencrypt_value,
};
use secrecy::ExposeSecret as _;

use super::callback::CallbackTransform;
use super::store::{CallbackRow, CallbackTable, MigrationStore, SettingsRow};

pub(super) struct Walkers<'run, S> {
    pub(super) store: &'run S,
    pub(super) cipher: &'run DbCipher,
}

impl<S: MigrationStore> Walkers<'_, S> {
    pub(super) async fn settings(
        &self,
        row: SettingsRow<'_>,
        dry_run: bool,
    ) -> Result<LocationReport, S::Error> {
        let mut report = LocationReport {
            location: row.location().to_owned(),
            scanned: 0,
            migrated: 0,
            already_v2: 0,
            plaintext: 0,
            undecryptable: 0,
            legacy: 0,
        };
        let Some(stored) = self.store.settings_row(row).await? else {
            return Ok(report);
        };
        let Some(mut settings) = mapping::<S>(stored)? else {
            return Ok(report);
        };
        let mut changed = false;
        for field in row_fields(row, &settings) {
            let outcome = migrate_field(&mut settings, &field, self.cipher, dry_run)
                .map_err(S::Error::from)?;
            if matches!(outcome, FieldOutcome::Migrated) {
                changed = true;
            }
            count_outcome::<S>(&mut report, outcome)?;
        }
        if changed {
            self.store
                .save_settings_row(row, &MigrationJson::Object(settings))
                .await?;
        }
        Ok(report)
    }

    pub(super) async fn callback_vars<T>(
        &self,
        table: CallbackTable,
        transform: &T,
        dry_run: bool,
    ) -> Result<LocationReport, S::Error>
    where
        T: CallbackTransform + Sync,
    {
        let mut report = LocationReport {
            location: table.location().to_owned(),
            scanned: 0,
            migrated: 0,
            already_v2: 0,
            plaintext: 0,
            undecryptable: 0,
            legacy: 0,
        };
        for row in self.store.callback_rows(table).await? {
            let counts = self
                .migrate_callback_row(table, transform, &row, dry_run)
                .await?;
            checked_add(&mut report.scanned, counts.scanned).map_err(S::Error::from)?;
            checked_add(&mut report.already_v2, counts.already_v2).map_err(S::Error::from)?;
            checked_add(&mut report.plaintext, counts.plaintext).map_err(S::Error::from)?;
            checked_add(&mut report.legacy, counts.legacy).map_err(S::Error::from)?;
            checked_add(&mut report.migrated, counts.migrated).map_err(S::Error::from)?;
            checked_add(&mut report.undecryptable, counts.undecryptable).map_err(S::Error::from)?;
        }
        Ok(report)
    }

    // A row's legacy total comes from classification alone, so it holds even when the transform later fails.
    async fn migrate_callback_row<T>(
        &self,
        table: CallbackTable,
        transform: &T,
        row: &CallbackRow,
        dry_run: bool,
    ) -> Result<CallbackCounts, S::Error>
    where
        T: CallbackTransform + Sync,
    {
        let Some(metadata) = row.metadata.as_ref() else {
            return Ok(CallbackCounts::default());
        };
        let mut counts = classify_row(metadata, self.cipher).map_err(S::Error::from)?;
        if counts.legacy == 0 || dry_run {
            return Ok(counts);
        }
        let Ok(re_encrypted) = transform.reencrypt(metadata, self.cipher) else {
            tracing::warn!(
                "Skipping {} callback_vars (transform failed)",
                table.location()
            );
            counts.undecryptable = counts.legacy;
            counts.legacy = 0;
            return Ok(counts);
        };
        counts.migrated = counts.legacy;
        counts.legacy = 0;
        self.store
            .save_callback_metadata(table, &row.key, &re_encrypted)
            .await?;
        Ok(counts)
    }
}

#[derive(Debug, Clone, Copy)]
enum FieldOutcome {
    Skipped,
    AlreadyV2,
    Plaintext,
    Residual,
    Migrated,
}

// A dry run reports a would-migrate legacy value as residual; a real run seals it and the caller rewrites the row.
fn migrate_field(
    settings: &mut BTreeMap<String, MigrationJson>,
    field: &str,
    cipher: &DbCipher,
    dry_run: bool,
) -> Result<FieldOutcome, ScanError> {
    let Some(value) = settings.get(field) else {
        return Ok(FieldOutcome::Skipped);
    };
    if matches!(value, MigrationJson::Null) {
        return Ok(FieldOutcome::Skipped);
    }
    let class = classify_value(value, cipher, field);
    if !matches!(class, ValueClass::Legacy) {
        return Ok(match class {
            ValueClass::Migrated => FieldOutcome::AlreadyV2,
            ValueClass::Plaintext | ValueClass::NotAString | ValueClass::Undecryptable => {
                FieldOutcome::Plaintext
            }
            ValueClass::Legacy => FieldOutcome::Residual,
        });
    }
    if dry_run {
        return Ok(FieldOutcome::Residual);
    }
    let MigrationJson::String(current) = value else {
        return Ok(FieldOutcome::Residual);
    };
    let replacement = reencrypt_value(value, cipher, field)?;
    let MigrationJson::String(sealed) = &replacement else {
        return Ok(FieldOutcome::Residual);
    };
    // The source keeps a legacy value that re-encrypts to the same ciphertext residual; a fresh nonce always changes it.
    if current.0.expose_secret() == sealed.0.expose_secret() {
        return Ok(FieldOutcome::Residual);
    }
    settings.insert(field.to_owned(), replacement);
    Ok(FieldOutcome::Migrated)
}

fn count_outcome<S>(report: &mut LocationReport, outcome: FieldOutcome) -> Result<(), S::Error>
where
    S: MigrationStore,
{
    match outcome {
        FieldOutcome::Skipped => Ok(()),
        FieldOutcome::AlreadyV2 => report
            .count(Some(ValueClass::Migrated))
            .map_err(S::Error::from),
        FieldOutcome::Plaintext => report
            .count(Some(ValueClass::Plaintext))
            .map_err(S::Error::from),
        FieldOutcome::Residual => report
            .count(Some(ValueClass::Legacy))
            .map_err(S::Error::from),
        FieldOutcome::Migrated => {
            checked_add(&mut report.scanned, 1).map_err(S::Error::from)?;
            checked_add(&mut report.migrated, 1).map_err(S::Error::from)
        }
    }
}

// Config rows migrate their listed keys; SSO migrates every non-empty string field it holds.
fn row_fields(row: SettingsRow<'_>, settings: &BTreeMap<String, MigrationJson>) -> Vec<String> {
    match row {
        SettingsRow::Config { fields, .. } => {
            fields.iter().map(|field| (*field).to_owned()).collect()
        }
        SettingsRow::Sso => settings
            .keys()
            .filter(|field| nonempty_string(settings, field.as_str()))
            .cloned()
            .collect(),
    }
}

fn nonempty_string(settings: &BTreeMap<String, MigrationJson>, field: &str) -> bool {
    match settings.get(field) {
        Some(MigrationJson::String(text)) => !text.0.expose_secret().is_empty(),
        Some(
            MigrationJson::Null
            | MigrationJson::Bool(_)
            | MigrationJson::Number(_)
            | MigrationJson::Array(_)
            | MigrationJson::Object(_),
        )
        | None => false,
    }
}

#[derive(Default)]
struct CallbackCounts {
    scanned: u64,
    already_v2: u64,
    plaintext: u64,
    legacy: u64,
    migrated: u64,
    undecryptable: u64,
}

// The row's legacy total comes from classification alone, so it holds even when the transform later fails.
fn classify_row(metadata: &MigrationJson, cipher: &DbCipher) -> Result<CallbackCounts, ScanError> {
    let mut counts = CallbackCounts::default();
    for vars in iter_callback_var_dicts(metadata) {
        for value in vars.values() {
            counts.scanned = counts
                .scanned
                .checked_add(1)
                .ok_or(ScanError::CounterOverflow)?;
            let counter = match classify_callback_value(value, cipher) {
                ValueClass::Legacy => &mut counts.legacy,
                ValueClass::Migrated => &mut counts.already_v2,
                ValueClass::Plaintext | ValueClass::NotAString | ValueClass::Undecryptable => {
                    &mut counts.plaintext
                }
            };
            *counter = counter.checked_add(1).ok_or(ScanError::CounterOverflow)?;
        }
    }
    Ok(counts)
}

// A JSON-string row is decoded once with B1602's decoder; a non-object result is ignored and invalid JSON propagates.
fn mapping<S>(stored: MigrationJson) -> Result<Option<BTreeMap<String, MigrationJson>>, S::Error>
where
    S: MigrationStore,
{
    let decoded = match stored {
        MigrationJson::Object(settings) => return Ok(Some(settings)),
        MigrationJson::String(text) => {
            decode_migration_json(text.0.expose_secret().as_bytes()).map_err(S::Error::from)?
        }
        MigrationJson::Null
        | MigrationJson::Bool(_)
        | MigrationJson::Number(_)
        | MigrationJson::Array(_) => {
            return Ok(None);
        }
    };
    match decoded {
        MigrationJson::Object(settings) => Ok(Some(settings)),
        MigrationJson::Null
        | MigrationJson::Bool(_)
        | MigrationJson::Number(_)
        | MigrationJson::String(_)
        | MigrationJson::Array(_) => Ok(None),
    }
}

fn checked_add(counter: &mut u64, amount: u64) -> Result<(), ScanError> {
    *counter = counter
        .checked_add(amount)
        .ok_or(ScanError::CounterOverflow)?;
    Ok(())
}
