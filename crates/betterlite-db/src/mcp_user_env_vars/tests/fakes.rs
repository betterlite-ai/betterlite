use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::{Level, SecretDiagnostics, SecretEvent};
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, NonceSource, StoredCipher, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};

use crate::budget_permission_rows::JsonData;
use crate::mcp::table_access::types::McpTableError;
use crate::postgres_store::errors::DbError;

use super::super::error::UserEnvVarsError;
use super::super::ports::{
    TransactionFuture, UserEnvVarTable, UserEnvVarsMergeStep, UserEnvVarsTransaction,
    UserEnvVarsTransactions,
};
use super::super::store::{UserEnvVars, UserEnvVarsStore};

pub(super) const SALT: &str = "b4714-salt";

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FakeRow {
    pub(super) user_id: String,
    pub(super) server_id: String,
    pub(super) values_b64: String,
}

impl FakeRow {
    pub(super) fn new(user_id: &str, server_id: &str, values_b64: String) -> Self {
        Self {
            user_id: user_id.to_owned(),
            server_id: server_id.to_owned(),
            values_b64,
        }
    }

    fn data(&self) -> JsonData {
        JsonData::Object(BTreeMap::from([
            ("user_id".to_owned(), JsonData::String(self.user_id.clone())),
            (
                "server_id".to_owned(),
                JsonData::String(self.server_id.clone()),
            ),
            (
                "values_b64".to_owned(),
                JsonData::String(self.values_b64.clone()),
            ),
        ]))
    }
}

#[derive(Default)]
struct FakeState {
    rows: Vec<FakeRow>,
    events: Vec<String>,
    locks: Vec<i64>,
    fail_at: Option<&'static str>,
}

impl FakeState {
    fn record(&mut self, event: &'static str) {
        self.events.push(event.to_owned());
    }

    fn fail(&self, event: &'static str) -> Result<(), UserEnvVarsError> {
        if self.fail_at == Some(event) {
            return Err(UserEnvVarsError::Database(McpTableError::from(
                DbError::NoDbConnected,
            )));
        }
        Ok(())
    }

    fn find_row(&self, user_id: &str, server_id: &str) -> Option<JsonData> {
        self.rows
            .iter()
            .find(|row| row.user_id == user_id && row.server_id == server_id)
            .map(FakeRow::data)
    }

    fn put_row(&mut self, user_id: &str, server_id: &str, values_b64: &str) {
        for row in &mut self.rows {
            if row.user_id == user_id && row.server_id == server_id {
                row.values_b64 = values_b64.to_owned();
                return;
            }
        }
        self.rows
            .push(FakeRow::new(user_id, server_id, values_b64.to_owned()));
    }
}

#[derive(Clone)]
pub(super) struct FakeTable {
    state: Arc<Mutex<FakeState>>,
}

impl FakeTable {
    pub(super) fn new(rows: Vec<FakeRow>) -> Self {
        Self::with_state(rows, None)
    }

    pub(super) fn failing(rows: Vec<FakeRow>, event: &'static str) -> Self {
        Self::with_state(rows, Some(event))
    }

    fn with_state(rows: Vec<FakeRow>, fail_at: Option<&'static str>) -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeState {
                rows,
                events: Vec::new(),
                locks: Vec::new(),
                fail_at,
            })),
        }
    }

    pub(super) fn events(&self) -> Vec<String> {
        lock(&self.state).events.clone()
    }

    pub(super) fn locks(&self) -> Vec<i64> {
        lock(&self.state).locks.clone()
    }

    pub(super) fn rows(&self) -> Vec<FakeRow> {
        lock(&self.state).rows.clone()
    }
}

impl UserEnvVarTable for FakeTable {
    async fn find_unique(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Option<JsonData>, UserEnvVarsError> {
        let mut state = lock(&self.state);
        state.record("find_unique");
        state.fail("find_unique")?;
        Ok(state.find_row(user_id, server_id))
    }

    async fn find_many(
        &self,
        user_id: &str,
        server_ids: &[String],
    ) -> Result<Vec<JsonData>, UserEnvVarsError> {
        let mut state = lock(&self.state);
        state.record("find_many");
        state.fail("find_many")?;
        Ok(state
            .rows
            .iter()
            .filter(|row| row.user_id == user_id && server_ids.contains(&row.server_id))
            .map(FakeRow::data)
            .collect())
    }

    async fn delete_many(&self, user_id: &str, server_id: &str) -> Result<usize, UserEnvVarsError> {
        let mut state = lock(&self.state);
        state.record("delete_many");
        state.fail("delete_many")?;
        let before = state.rows.len();
        state
            .rows
            .retain(|row| row.user_id != user_id || row.server_id != server_id);
        Ok(before.saturating_sub(state.rows.len()))
    }
}

struct FakeTransaction {
    state: Arc<Mutex<FakeState>>,
}

impl UserEnvVarsTransaction for FakeTransaction {
    fn execute_raw<'a>(&'a self, _statement: &'a str, key: i64) -> TransactionFuture<'a, ()> {
        Box::pin(async move {
            let mut state = lock(&self.state);
            state.record("lock");
            state.fail("lock")?;
            state.locks.push(key);
            Ok(())
        })
    }

    fn find_unique<'a>(
        &'a self,
        user_id: &'a str,
        server_id: &'a str,
    ) -> TransactionFuture<'a, Option<JsonData>> {
        Box::pin(async move {
            let mut state = lock(&self.state);
            state.record("find_unique");
            state.fail("find_unique")?;
            Ok(state.find_row(user_id, server_id))
        })
    }

    fn upsert<'a>(
        &'a self,
        user_id: &'a str,
        server_id: &'a str,
        values_b64: &'a str,
    ) -> TransactionFuture<'a, ()> {
        Box::pin(async move {
            let mut state = lock(&self.state);
            state.record("upsert");
            state.fail("upsert")?;
            state.put_row(user_id, server_id, values_b64);
            Ok(())
        })
    }
}

impl UserEnvVarsTransactions for FakeTable {
    fn in_transaction<'a>(
        &'a self,
        step: &'a dyn UserEnvVarsMergeStep,
    ) -> TransactionFuture<'a, UserEnvVars> {
        lock(&self.state).record("tx");
        let locked = FakeTransaction {
            state: Arc::clone(&self.state),
        };
        Box::pin(async move { step.apply(&locked).await })
    }
}

pub(super) struct FixedNonces;

impl NonceSource for FixedNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        nonce.fill(0x2a);
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct SilentDiagnostics;

impl SecretDiagnostics for SilentDiagnostics {
    fn record(&self, _level: Level, _event: &SecretEvent<'_>) {}
}

#[derive(Clone, Default)]
pub(super) struct RecordingDiagnostics(Arc<Mutex<Vec<(Level, String)>>>);

impl RecordingDiagnostics {
    pub(super) fn events(&self) -> Vec<(Level, String)> {
        lock(&self.0).clone()
    }
}

impl SecretDiagnostics for RecordingDiagnostics {
    fn record(&self, level: Level, event: &SecretEvent<'_>) {
        lock(&self.0).push((level, event.message()));
    }
}

pub(super) type TestCipher = StoredCipher<FixedNonces, SilentDiagnostics>;

pub(super) type TestStore = UserEnvVarsStore<FakeTable, FakeTable, FixedNonces, SilentDiagnostics>;

pub(super) const fn unkeyed_cipher() -> TestCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: None,
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: FixedNonces,
        diagnostics: SilentDiagnostics,
    }
}

fn keyed_settings(salt: &str) -> CipherSettings {
    CipherSettings {
        signing_key: Some(SecretString::from(salt.to_owned())),
        algorithm: Algorithm::XSalsa20Poly1305,
    }
}

pub(super) fn test_cipher(salt: &str) -> TestCipher {
    StoredCipher {
        settings: keyed_settings(salt),
        nonces: FixedNonces,
        diagnostics: SilentDiagnostics,
    }
}

pub(super) fn recording_cipher(
    salt: &str,
) -> (
    StoredCipher<FixedNonces, RecordingDiagnostics>,
    RecordingDiagnostics,
) {
    let diagnostics = RecordingDiagnostics::default();
    let cipher = StoredCipher {
        settings: keyed_settings(salt),
        nonces: FixedNonces,
        diagnostics: diagnostics.clone(),
    };
    (cipher, diagnostics)
}

pub(super) fn test_store(table: &FakeTable) -> TestStore {
    test_store_with(table, test_cipher(SALT))
}

pub(super) fn test_store_with(table: &FakeTable, cipher: TestCipher) -> TestStore {
    UserEnvVarsStore {
        table: table.clone(),
        transactions: table.clone(),
        cipher,
    }
}

pub(super) fn sealed(cipher: &TestCipher, plaintext: &str) -> String {
    let value = StoredValue::<()>::Text(SecretString::from(plaintext.to_owned()));
    match cipher
        .encrypt_value(value, None)
        .expect("the fixed test cipher seals")
    {
        StoredValue::Text(text) => text.expose_secret().to_owned(),
        StoredValue::Other(()) => panic!("a text value stays text"),
    }
}
