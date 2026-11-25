#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_db::encryption_scan::report::{LocationReport, MigrationReport};
use betterlite_db::encryption_scan::secret_text::SecretText;
use betterlite_db::encryption_scan::{DbCipher, MigrationJson, ScanError};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::Error as _;

use super::super::callback::{CallbackTransform, CallbackVars};
use super::super::covered::SameKeyRotation;
use super::super::service::Migration;
use super::super::store::{CallbackRow, CallbackTable, MigrationStore};

pub(super) const SALT_KEY: &str = "configured-salt-key";
pub(super) const COVERED: [&str; 6] = [
    "model_table",
    "credentials",
    "mcp_server",
    "mcp_user_credentials",
    "mcp_user_env_vars",
    "config_environment_variables",
];

pub(super) fn cipher(algorithm: Algorithm, key: Option<&str>) -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: key.map(|value| SecretString::from(value.to_owned())),
            algorithm,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(super) fn sealed(algorithm: Algorithm, key: &str, text: &str) -> String {
    let stored = StoredValue::<()>::Text(SecretString::from(text.to_owned()));
    let StoredValue::Text(ciphertext) = cipher(algorithm, Some(key))
        .encrypt_value(stored, None)
        .expect("the fixture value seals")
    else {
        panic!("a string seals as text");
    };
    ciphertext.expose_secret().to_owned()
}

pub(super) fn legacy(key: &str, text: &str) -> String {
    sealed(Algorithm::XSalsa20Poly1305, key, text)
}

pub(super) fn v2(key: &str, text: &str) -> String {
    sealed(Algorithm::Aes256Gcm, key, text)
}

pub(super) fn marked(value: &str) -> String {
    format!("litellm_enc::{value}")
}

pub(super) fn open(key: &str, text: &str) -> Option<String> {
    let stored = StoredValue::<()>::Text(SecretString::from(text.to_owned()));
    let policy = DecryptPolicy {
        log: FailureLog::Debug,
        value: FailureValue::Nothing,
    };
    match cipher(Algorithm::XSalsa20Poly1305, Some(key)).decrypt_value(stored, "test", policy) {
        Some(StoredValue::Text(plain)) => Some(plain.expose_secret().to_owned()),
        Some(StoredValue::Other(())) | None => None,
    }
}

pub(super) fn object(fields: Vec<(&str, MigrationJson)>) -> MigrationJson {
    MigrationJson::Object(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

pub(super) fn secret(text: &str) -> MigrationJson {
    MigrationJson::String(SecretText(SecretString::from(text.to_owned())))
}

pub(super) fn callback_row(key: &str, metadata: MigrationJson) -> CallbackRow {
    CallbackRow {
        key: key.to_owned(),
        metadata: Some(metadata),
    }
}

fn callback_vars(vars: Vec<(&str, String)>) -> MigrationJson {
    object(
        vars.into_iter()
            .map(|(name, value)| (name, secret(&value)))
            .collect(),
    )
}

pub(super) fn callback_vars_metadata(vars: Vec<(&str, String)>) -> MigrationJson {
    let entry = object(vec![("callback_vars", callback_vars(vars))]);
    object(vec![("logging", MigrationJson::Array(vec![entry]))])
}

pub(super) fn callback_settings_metadata(vars: Vec<(&str, String)>) -> MigrationJson {
    let settings = object(vec![("callback_vars", callback_vars(vars))]);
    object(vec![("callback_settings", settings)])
}

pub(super) fn callback_text(metadata: &MigrationJson, name: &str) -> String {
    let MigrationJson::Object(fields) = metadata else {
        panic!("callback metadata is an object");
    };
    let Some(MigrationJson::Array(entries)) = fields.get("logging") else {
        panic!("callback metadata carries logging");
    };
    let Some(MigrationJson::Object(entry)) = entries.first() else {
        panic!("logging carries an entry");
    };
    let Some(MigrationJson::Object(vars)) = entry.get("callback_vars") else {
        panic!("the entry carries callback_vars");
    };
    let Some(MigrationJson::String(text)) = vars.get(name) else {
        panic!("callback_vars carries {name}");
    };
    text.0.expose_secret().to_owned()
}

pub(super) fn field_text(value: &MigrationJson, name: &str) -> Option<String> {
    let MigrationJson::Object(fields) = value else {
        return None;
    };
    match fields.get(name) {
        Some(MigrationJson::String(text)) => Some(text.0.expose_secret().to_owned()),
        Some(
            MigrationJson::Null
            | MigrationJson::Bool(_)
            | MigrationJson::Number(_)
            | MigrationJson::Array(_)
            | MigrationJson::Object(_),
        )
        | None => None,
    }
}

pub(super) fn location(name: &str) -> LocationReport {
    LocationReport {
        location: name.to_owned(),
        scanned: 0,
        migrated: 0,
        already_v2: 0,
        plaintext: 0,
        undecryptable: 0,
        legacy: 0,
    }
}

pub(super) fn residual(name: &str, legacy: u64) -> LocationReport {
    LocationReport {
        scanned: legacy,
        legacy,
        ..location(name)
    }
}

pub(super) fn report(locations: Vec<LocationReport>) -> MigrationReport {
    MigrationReport { locations }
}

pub(super) fn location_of<'a>(report: &'a MigrationReport, name: &str) -> &'a LocationReport {
    report
        .locations
        .iter()
        .find(|location| location.location == name)
        .expect("the report carries the location")
}

pub(super) fn json(value: &MigrationJson) -> String {
    serde_json::to_string(value).expect("the fixture value is JSON")
}

pub(super) fn covered_report(legacy: [u64; 6]) -> MigrationReport {
    report(
        COVERED
            .iter()
            .zip(legacy)
            .map(|(name, count)| residual(name, count))
            .collect(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Failure {
    ReadConfig,
    SaveConfig,
    ReadSso,
    SaveSso,
    ReadCallbacks,
    SaveCallback,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum FakeError {
    #[error("the fake store failed on {0:?}")]
    Failed(Failure),
    #[error("the fake store has no queued scan")]
    NoScan,
    #[error(transparent)]
    Scan(#[from] ScanError),
}

#[derive(Debug, Clone)]
pub(super) enum Write {
    Config {
        param_name: String,
        value: MigrationJson,
    },
    Sso(MigrationJson),
    Callback {
        table: CallbackTable,
        key: String,
        metadata: MigrationJson,
    },
}

#[derive(Default)]
pub(super) struct FakeStore {
    pub scans: Mutex<VecDeque<MigrationReport>>,
    pub configs: Mutex<BTreeMap<String, MigrationJson>>,
    pub sso: Mutex<Option<MigrationJson>>,
    pub teams: Mutex<Vec<CallbackRow>>,
    pub tokens: Mutex<Vec<CallbackRow>>,
    pub failures: Mutex<Vec<Failure>>,
    pub writes: Mutex<Vec<Write>>,
}

impl FakeStore {
    pub(super) fn writes(&self) -> Vec<Write> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn queued_scans(&self) -> Vec<MigrationReport> {
        self.scans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    fn fail(&self, failure: Failure) -> Result<(), FakeError> {
        let failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        if failures.contains(&failure) {
            return Err(FakeError::Failed(failure));
        }
        Ok(())
    }

    fn record(&self, write: Write) {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(write);
    }

    fn callback_rows_for(&self, table: CallbackTable) -> Vec<CallbackRow> {
        let rows = match table {
            CallbackTable::Team => &self.teams,
            CallbackTable::VerificationToken => &self.tokens,
        };
        rows.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl MigrationStore for FakeStore {
    type Error = FakeError;

    fn scan_covered(
        &self,
        _cipher: &DbCipher,
    ) -> impl Future<Output = Result<MigrationReport, FakeError>> + Send {
        let report = self
            .scans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        ready(report.ok_or(FakeError::NoScan))
    }

    fn config_row(
        &self,
        param_name: &str,
    ) -> impl Future<Output = Result<Option<MigrationJson>, FakeError>> + Send {
        let found = self.fail(Failure::ReadConfig).map(|()| {
            self.configs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(param_name)
                .cloned()
        });
        ready(found)
    }

    fn save_config_row(
        &self,
        param_name: &str,
        value: &MigrationJson,
    ) -> impl Future<Output = Result<(), FakeError>> + Send {
        match self.fail(Failure::SaveConfig) {
            Ok(()) => {
                self.record(Write::Config {
                    param_name: param_name.to_owned(),
                    value: value.clone(),
                });
                ready(Ok(()))
            }
            Err(error) => ready(Err(error)),
        }
    }

    fn sso_row(&self) -> impl Future<Output = Result<Option<MigrationJson>, FakeError>> + Send {
        let found = self.fail(Failure::ReadSso).map(|()| {
            self.sso
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        });
        ready(found)
    }

    fn save_sso_row(
        &self,
        value: &MigrationJson,
    ) -> impl Future<Output = Result<(), FakeError>> + Send {
        match self.fail(Failure::SaveSso) {
            Ok(()) => {
                self.record(Write::Sso(value.clone()));
                ready(Ok(()))
            }
            Err(error) => ready(Err(error)),
        }
    }

    fn callback_rows(
        &self,
        table: CallbackTable,
    ) -> impl Future<Output = Result<Vec<CallbackRow>, FakeError>> + Send {
        let checked = self.fail(Failure::ReadCallbacks);
        ready(checked.map(|()| self.callback_rows_for(table)))
    }

    fn save_callback_metadata(
        &self,
        table: CallbackTable,
        key: &str,
        metadata: &MigrationJson,
    ) -> impl Future<Output = Result<(), FakeError>> + Send {
        match self.fail(Failure::SaveCallback) {
            Ok(()) => {
                self.record(Write::Callback {
                    table,
                    key: key.to_owned(),
                    metadata: metadata.clone(),
                });
                ready(Ok(()))
            }
            Err(error) => ready(Err(error)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RotationCall {
    pub key: String,
    pub actor: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("the fake rotation failed")]
pub(super) struct RotationFailure;

#[derive(Default)]
pub(super) struct FakeRotation {
    pub calls: Mutex<Vec<RotationCall>>,
    pub failure: bool,
}

impl FakeRotation {
    pub(super) fn calls(&self) -> Vec<RotationCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SameKeyRotation for FakeRotation {
    type Error = RotationFailure;

    fn reencrypt_same_key(
        &mut self,
        key: &SecretString,
        actor: Option<&str>,
    ) -> impl Future<Output = Result<(), RotationFailure>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RotationCall {
                key: key.expose_secret().to_owned(),
                actor: actor.map(str::to_owned),
            });
        if self.failure {
            return ready(Err(RotationFailure));
        }
        ready(Ok(()))
    }
}

pub(super) struct FailingTransform;

impl CallbackTransform for FailingTransform {
    fn reencrypt(
        &self,
        _metadata: &MigrationJson,
        _cipher: &DbCipher,
    ) -> Result<MigrationJson, serde_json::Error> {
        Err(serde_json::Error::custom("the callback transform fails"))
    }
}

pub(super) fn migration<'run>(
    store: &'run FakeStore,
    rotation: &'run mut FakeRotation,
    cipher: &'run DbCipher,
    dry_run: bool,
) -> Migration<'run, FakeStore, FakeRotation, CallbackVars> {
    Migration {
        store,
        rotation,
        cipher,
        transform: CallbackVars,
        actor: Some("admin"),
        dry_run,
    }
}
