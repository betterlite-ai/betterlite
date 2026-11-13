use std::collections::VecDeque;
use std::future::{ready, Future};
use std::sync::{Arc, Mutex};

use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    decrypt_if_encrypted_with, Algorithm, CipherSettings, OsNonces, StoredValue,
    CALLBACK_VAR_ENCRYPTED_PREFIX,
};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::Notify;

use crate::database_reencryption::registry::{registered_columns, SecretColumn};
use crate::database_reencryption::runner::MigrationRequest;
use crate::database_reencryption::storage::{SecretRow, SecretStorage};
use deadpool_postgres::PoolError;

use crate::database_reencryption::MigrationError;
use crate::encryption_scan::{decode_migration_json, iter_encrypted_strings, MigrationJson};
use crate::postgres_store::diagnostics::PoolFailure;
use crate::stored_cipher::DbCipher;

pub(crate) const PREVIOUS_KEY: &str = "sk-previous-rotation-key";
pub(crate) const MASTER_KEY: &str = "sk-new-master-key";
pub(crate) const SALT_KEY: &str = "sk-configured-salt";

pub(crate) type Events = Arc<Mutex<Vec<String>>>;

pub(crate) fn events() -> Events {
    Arc::new(Mutex::new(Vec::new()))
}

pub(crate) fn recorded(events: &Events) -> Vec<String> {
    events.lock().expect("the events lock").clone()
}

pub(crate) fn cipher(algorithm: Algorithm) -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(SALT_KEY)),
            algorithm,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(crate) fn sealed(plaintext: &str, key: &str, algorithm: Algorithm) -> String {
    let encrypted = cipher(algorithm)
        .encrypt_value(
            StoredValue::<()>::Text(SecretString::from(plaintext)),
            Some(&SecretString::from(key)),
        )
        .expect("the fixture encrypts");
    let StoredValue::Text(text) = encrypted else {
        panic!("a text value encrypts to text")
    };
    text.expose_secret().to_owned()
}

pub(crate) fn json(text: &str) -> MigrationJson {
    decode_migration_json(text.as_bytes()).expect("the fixture is JSON")
}

pub(crate) fn row(primary_key: &str, value: &str) -> SecretRow {
    SecretRow {
        primary_key: primary_key.to_owned(),
        value: Some(json(value)),
    }
}

pub(crate) fn column(table: &str, name: &str) -> SecretColumn {
    registered_columns()
        .find(|candidate| candidate.table == table && candidate.column == name)
        .expect("the column is registered")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Replaced {
    pub(crate) column: String,
    pub(crate) old: String,
    pub(crate) new: String,
    pub(crate) primary_key: String,
}

pub(crate) struct FakeStorage {
    pub(crate) events: Events,
    pub(crate) columns: Vec<SecretColumn>,
    pub(crate) replaced: Mutex<Vec<Replaced>>,
    pub(crate) block: Option<Arc<Notify>>,
    pub(crate) batches: Mutex<VecDeque<Result<Vec<SecretRow>, MigrationError>>>,
    pub(crate) updates: Mutex<VecDeque<bool>>,
}

impl FakeStorage {
    pub(crate) fn new(events: &Events, columns: Vec<SecretColumn>) -> Self {
        Self {
            events: Arc::clone(events),
            columns,
            replaced: Mutex::new(Vec::new()),
            block: None,
            batches: Mutex::new(VecDeque::new()),
            updates: Mutex::new(VecDeque::new()),
        }
    }

    pub(crate) fn block_on_read(mut self, gate: Arc<Notify>) -> Self {
        self.block = Some(gate);
        self
    }

    pub(crate) fn script_rows(&self, rows: Vec<SecretRow>) {
        self.batches
            .lock()
            .expect("the batches lock")
            .push_back(Ok(rows));
    }

    pub(crate) fn script_unavailable(&self) {
        self.batches
            .lock()
            .expect("the batches lock")
            .push_back(Err(MigrationError::Connect(PoolFailure {
                driver: PoolError::Closed,
            })));
    }

    pub(crate) fn script_update(&self, updated: bool) {
        self.updates
            .lock()
            .expect("the updates lock")
            .push_back(updated);
    }

    pub(crate) fn replaced(&self) -> Vec<Replaced> {
        self.replaced.lock().expect("the replaced lock").clone()
    }
}

impl SecretStorage for FakeStorage {
    fn secret_columns(
        &self,
    ) -> impl Future<Output = Result<Vec<SecretColumn>, MigrationError>> + Send {
        self.events
            .lock()
            .expect("the events lock")
            .push("columns".to_owned());
        ready(Ok(self.columns.clone()))
    }

    async fn rows(&self, column: &SecretColumn) -> Result<Vec<SecretRow>, MigrationError> {
        self.events
            .lock()
            .expect("the events lock")
            .push(format!("rows:{}", column.column));
        if let Some(gate) = &self.block {
            gate.notified().await;
        }
        self.batches
            .lock()
            .expect("the batches lock")
            .pop_front()
            .unwrap_or_else(|| Ok(Vec::new()))
    }

    fn replace(
        &self,
        column: &SecretColumn,
        old: &str,
        new: &str,
        primary_key: &str,
    ) -> impl Future<Output = Result<bool, MigrationError>> + Send {
        self.events
            .lock()
            .expect("the events lock")
            .push(format!("replace:{primary_key}"));
        self.replaced
            .lock()
            .expect("the replaced lock")
            .push(Replaced {
                column: column.column.to_owned(),
                old: old.to_owned(),
                new: new.to_owned(),
                primary_key: primary_key.to_owned(),
            });
        let updated = self
            .updates
            .lock()
            .expect("the updates lock")
            .pop_front()
            .unwrap_or(false);
        ready(Ok(updated))
    }
}

pub(crate) fn request<'keys>(
    previous_master_key: Option<&'keys SecretString>,
    master_key: Option<&'keys SecretString>,
    salt_key_is_set: bool,
    cipher: &'keys DbCipher,
) -> MigrationRequest<'keys> {
    MigrationRequest {
        previous_master_key,
        master_key,
        salt_key_is_set,
        cipher,
        max_depth: 100,
    }
}

pub(crate) fn log_into(
    events: &Events,
) -> impl Fn(&str) -> Result<(), MigrationError> + Send + Sync {
    let captured = Arc::clone(events);
    move |message| {
        captured
            .lock()
            .expect("the events lock")
            .push(format!("log:{message}"));
        Ok(())
    }
}

pub(crate) fn decrypted_leaves(value: &MigrationJson, key: &str) -> Vec<String> {
    let signing_key = SecretString::from(key);
    iter_encrypted_strings(value)
        .filter_map(|text| {
            let exposed = text.0.expose_secret();
            let inner = exposed
                .strip_prefix(CALLBACK_VAR_ENCRYPTED_PREFIX)
                .unwrap_or(exposed);
            decrypt_if_encrypted_with(inner, &signing_key)
                .map(|plain| plain.expose_secret().to_owned())
        })
        .collect()
}

pub(crate) fn steps(events: &[String]) -> Vec<String> {
    events
        .iter()
        .map(|event| {
            let text = event.as_str();
            if text == "columns" {
                String::from("columns")
            } else if text.starts_with("rows:") {
                String::from("rows")
            } else if text.starts_with("replace:") {
                String::from("replace")
            } else if text.starts_with("log:Re-encrypting") {
                String::from("progress")
            } else if text.starts_with("log:") {
                String::from("outcome")
            } else {
                text.to_owned()
            }
        })
        .collect()
}

pub(crate) fn keys() -> (SecretString, SecretString) {
    (
        SecretString::from(PREVIOUS_KEY),
        SecretString::from(MASTER_KEY),
    )
}
