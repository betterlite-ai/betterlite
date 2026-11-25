use std::collections::BTreeMap;

use betterlite_db::encryption_scan::report::{LocationReport, ValueClass};
use betterlite_db::encryption_scan::{
    DbCipher, MigrationJson, ScanError, classify_callback_value, classify_value,
    decode_migration_json, iter_callback_var_dicts, reencrypt_value,
};
use secrecy::ExposeSecret as _;

use super::callback::CallbackTransform;
use super::store::{CallbackTable, MigrationStore};

const LOGGING: &str = "logging";
const CALLBACK_SETTINGS: &str = "callback_settings";
const SSO_CONFIG: &str = "sso_config";

pub(super) struct Walkers<'run, S> {
    pub(super) store: &'run S,
    pub(super) cipher: &'run DbCipher,
}

impl<S: MigrationStore> Walkers<'_, S> {
    pub(super) async fn config_settings(
        &self,
        param_name: &str,
        fields: &[&str],
        dry_run: bool,
    ) -> Result<LocationReport, S::Error> {
        let mut report = empty_report(param_name);
        let Some(stored) = self.store.config_row(param_name).await? else {
            return Ok(report);
        };
        let Some(mut settings) = mapping::<S>(stored)? else {
            return Ok(report);
        };
        let mut changed = false;
        for field in fields {
            let outcome = migrate_field(&mut settings, field, self.cipher, dry_run)
                .map_err(scan_error::<S>)?;
            if matches!(outcome, FieldOutcome::Migrated) {
                changed = true;
            }
            count_outcome::<S>(&mut report, outcome)?;
        }
        if changed {
            self.store
                .save_config_row(param_name, &MigrationJson::Object(settings))
                .await?;
        }
        Ok(report)
    }

    pub(super) async fn sso_config(&self, dry_run: bool) -> Result<LocationReport, S::Error> {
        let mut report = empty_report(SSO_CONFIG);
        let Some(stored) = self.store.sso_row().await? else {
            return Ok(report);
        };
        let Some(mut settings) = mapping::<S>(stored)? else {
            return Ok(report);
        };
        let fields = settings.keys().cloned().collect::<Vec<_>>();
        let mut changed = false;
        for field in fields {
            if !nonempty_string(&settings, &field) {
                continue;
            }
            let outcome = migrate_field(&mut settings, &field, self.cipher, dry_run)
                .map_err(scan_error::<S>)?;
            if matches!(outcome, FieldOutcome::Migrated) {
                changed = true;
            }
            count_outcome::<S>(&mut report, outcome)?;
        }
        if changed {
            self.store
                .save_sso_row(&MigrationJson::Object(settings))
                .await?;
        }
        Ok(report)
    }

    pub(super) async fn callback_vars<T: CallbackTransform>(
        &self,
        table: CallbackTable,
        transform: &T,
        dry_run: bool,
    ) -> Result<LocationReport, S::Error> {
        let mut report = empty_report(table.location());
        for row in self.store.callback_rows(table).await? {
            let Some(metadata) = row.metadata.as_ref() else {
                continue;
            };
            if !has_callback_slots(metadata) {
                continue;
            }
            let counts = classify_row(metadata, self.cipher).map_err(scan_error::<S>)?;
            checked_add(&mut report.scanned, counts.scanned).map_err(scan_error::<S>)?;
            checked_add(&mut report.already_v2, counts.already_v2).map_err(scan_error::<S>)?;
            checked_add(&mut report.plaintext, counts.plaintext).map_err(scan_error::<S>)?;
            if counts.legacy == 0 {
                continue;
            }
            if dry_run {
                checked_add(&mut report.legacy, counts.legacy).map_err(scan_error::<S>)?;
                continue;
            }
            let Ok(re_encrypted) = transform.reencrypt(metadata, self.cipher) else {
                tracing::warn!(
                    "Skipping {} callback_vars (transform failed)",
                    table.location()
                );
                checked_add(&mut report.undecryptable, counts.legacy).map_err(scan_error::<S>)?;
                continue;
            };
            checked_add(&mut report.migrated, counts.legacy).map_err(scan_error::<S>)?;
            self.store
                .save_callback_metadata(table, &row.key, &re_encrypted)
                .await?;
        }
        Ok(report)
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
    let replacement = reencrypt_value(value, cipher, field)?;
    settings.insert(field.to_owned(), replacement);
    Ok(FieldOutcome::Migrated)
}

fn count_outcome<S: MigrationStore>(
    report: &mut LocationReport,
    outcome: FieldOutcome,
) -> Result<(), S::Error> {
    match outcome {
        FieldOutcome::Skipped => Ok(()),
        FieldOutcome::AlreadyV2 => report
            .count(Some(ValueClass::Migrated))
            .map_err(scan_error::<S>),
        FieldOutcome::Plaintext => report
            .count(Some(ValueClass::Plaintext))
            .map_err(scan_error::<S>),
        FieldOutcome::Residual => report
            .count(Some(ValueClass::Legacy))
            .map_err(scan_error::<S>),
        FieldOutcome::Migrated => {
            checked_add(&mut report.scanned, 1).map_err(scan_error::<S>)?;
            checked_add(&mut report.migrated, 1).map_err(scan_error::<S>)
        }
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

fn has_callback_slots(metadata: &MigrationJson) -> bool {
    let MigrationJson::Object(fields) = metadata else {
        return false;
    };
    fields.contains_key(LOGGING) || fields.contains_key(CALLBACK_SETTINGS)
}

#[derive(Default)]
struct RowCounts {
    scanned: u64,
    already_v2: u64,
    plaintext: u64,
    legacy: u64,
}

// The row's legacy total comes from classification alone, so it holds even when the transform later fails.
fn classify_row(metadata: &MigrationJson, cipher: &DbCipher) -> Result<RowCounts, ScanError> {
    let mut counts = RowCounts::default();
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
fn mapping<S: MigrationStore>(
    stored: MigrationJson,
) -> Result<Option<BTreeMap<String, MigrationJson>>, S::Error> {
    let decoded = match stored {
        MigrationJson::Object(settings) => return Ok(Some(settings)),
        MigrationJson::String(text) => decode(text.0.expose_secret().as_bytes())?,
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

fn decode<S: MigrationStore>(bytes: &[u8]) -> Result<MigrationJson, S::Error> {
    decode_migration_json(bytes).map_err(scan_error::<S>)
}

fn scan_error<S: MigrationStore>(source: ScanError) -> S::Error {
    <S::Error as From<ScanError>>::from(source)
}

fn checked_add(counter: &mut u64, amount: u64) -> Result<(), ScanError> {
    *counter = counter
        .checked_add(amount)
        .ok_or(ScanError::CounterOverflow)?;
    Ok(())
}

fn empty_report(location: &str) -> LocationReport {
    LocationReport {
        location: location.to_owned(),
        scanned: 0,
        migrated: 0,
        already_v2: 0,
        plaintext: 0,
        undecryptable: 0,
        legacy: 0,
    }
}
