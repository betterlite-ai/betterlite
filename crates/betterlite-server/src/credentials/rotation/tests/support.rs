#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::str::from_utf8;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::model_secrets::PreparedModel;
use betterlite_db::resource_repository::config::ConfigParam;
use betterlite_db::resource_rows::credential::CredentialItem as CredentialRow;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_db::sso_assertion_rows::{AssertionRow, MemoryAssertionRows};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::super::Rotation;
use super::super::sink::ModelTableInvalidation;
use super::super::store::RotationStore;
use crate::config_override_support::environment::RuntimeEnvironment;
use crate::db_row_decrypt::warnings::Warnings;
use crate::mcp_rotation::store::{
    McpRotationStore, ServerUpdate, StoredOAuthClientRow, StoredServerRow, StoredUserCredentialRow,
    StoredUserEnvVarRow,
};

pub(super) const OLD_KEY: &str = "old-master-key";
pub(super) const NEW_KEY: &str = "new-master-key";

const UNREADABLE: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Nothing,
};

pub(super) fn cipher(key: &str) -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(key.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(super) fn new_key() -> SecretString {
    SecretString::from(NEW_KEY)
}

pub(super) fn sealed(text: &str) -> String {
    sealed_by(OLD_KEY, text)
}

// a value sealed with another key is what a rotation has to leave alone
pub(super) fn sealed_by(key: &str, text: &str) -> String {
    let stored = StoredValue::<()>::Text(SecretString::from(text.to_owned()));
    let StoredValue::Text(ciphertext) = cipher(key)
        .encrypt_value(stored, None)
        .expect("the fixture value seals")
    else {
        panic!("a string seals as text");
    };
    ciphertext.expose_secret().to_owned()
}

pub(super) fn open(text: &str, key: &str) -> Option<String> {
    let stored = StoredValue::<()>::Text(SecretString::from(text.to_owned()));
    match cipher(key).decrypt_value(stored, "rotation", UNREADABLE) {
        Some(StoredValue::Text(plain)) => Some(plain.expose_secret().to_owned()),
        Some(StoredValue::Other(())) | None => None,
    }
}

struct NoOidcHttp;

impl OidcHttp for NoOidcHttp {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "a rotation test has no OIDC provider".to_owned(),
        })
    }
}

struct NoAzureTokens;

impl AzureOidcTokens for NoAzureTokens {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

struct FixedClock;

impl OidcClock for FixedClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidcHttp),
        azure_tokens: Arc::new(NoAzureTokens),
        clock: Arc::new(FixedClock),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) fn text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

pub(super) fn object(fields: Vec<(&str, JsonData)>) -> JsonData {
    JsonData::Object(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

pub(super) fn model_row(model_id: &str, params: Vec<(&str, JsonData)>) -> ProxyModelTable {
    let mut fields = BTreeMap::new();
    fields.insert("model_id".to_owned(), text(model_id));
    fields.insert("model_name".to_owned(), text("gpt-4o"));
    fields.insert("litellm_params".to_owned(), object(params));
    ProxyModelTable::try_from(JsonData::Object(fields)).expect("the fixture model row decodes")
}

pub(super) fn config_row(name: &str, value: JsonData) -> ConfigParam {
    ConfigParam {
        param_name: name.to_owned(),
        param_value: value,
    }
}

pub(super) fn credential_row(name: &str, values: Vec<(&str, JsonData)>) -> JsonData {
    credential_with(name, Vec::new(), values)
}

pub(super) fn credential_with(
    name: &str,
    info: Vec<(&str, JsonData)>,
    values: Vec<(&str, JsonData)>,
) -> JsonData {
    object(vec![
        ("credential_name", text(name)),
        ("credential_info", object(info)),
        ("credential_values", object(values)),
    ])
}

pub(super) fn assertion_row(user_id: &str, assertion: &str) -> AssertionRow {
    AssertionRow {
        user_id: user_id.to_owned(),
        assertion_b64: sealed(assertion),
    }
}

pub(super) fn server_row(server_id: &str, blob: String) -> StoredServerRow {
    StoredServerRow {
        server_id: server_id.to_owned(),
        credentials: Some(JsonValue::String(blob)),
        env_vars: None,
        static_headers: None,
        env: None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Write {
    Models(Vec<PreparedModel>),
    Environment(JsonData),
    Credential {
        name: String,
        row: CredentialRow,
        actor: Option<String>,
    },
    Publish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Failure {
    ModelScan,
    ConfigScan,
    CredentialScan,
    SaveModels,
    SaveEnvironment,
    CredentialUpdate,
}

#[derive(Debug, thiserror::Error)]
#[error("the fake store failed on {failure:?}")]
pub(super) struct FakeStoreError {
    pub failure: Failure,
}

pub(super) type Log = Arc<Mutex<Vec<Write>>>;

#[derive(Default)]
pub(super) struct FakeStore {
    pub models: Vec<ProxyModelTable>,
    pub config: Vec<ConfigParam>,
    pub credentials: Vec<JsonData>,
    pub failures: Vec<Failure>,
    pub rejected: Vec<String>,
    pub log: Log,
}

impl FakeStore {
    pub(super) fn writes(&self) -> Vec<Write> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn fail(&self, failure: Failure) -> Result<(), FakeStoreError> {
        if self.failures.contains(&failure) {
            return Err(FakeStoreError { failure });
        }
        Ok(())
    }

    fn record(&self, write: Write) {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(write);
    }
}

impl RotationStore for FakeStore {
    type Error = FakeStoreError;

    fn model_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<ProxyModelTable>, FakeStoreError>> + Send {
        ready(self.fail(Failure::ModelScan).map(|()| self.models.clone()))
    }

    fn save_model_params(
        &self,
        models: &[PreparedModel],
    ) -> impl Future<Output = Result<(), FakeStoreError>> + Send {
        if let Err(error) = self.fail(Failure::SaveModels) {
            return ready(Err(error));
        }
        self.record(Write::Models(models.to_vec()));
        ready(Ok(()))
    }

    fn config_rows(&self) -> impl Future<Output = Result<Vec<ConfigParam>, FakeStoreError>> + Send {
        ready(self.fail(Failure::ConfigScan).map(|()| self.config.clone()))
    }

    fn save_environment_variables(
        &self,
        values: JsonData,
    ) -> impl Future<Output = Result<(), FakeStoreError>> + Send {
        match self.fail(Failure::SaveEnvironment) {
            Ok(()) => {
                self.record(Write::Environment(values));
                ready(Ok(()))
            }
            Err(error) => ready(Err(error)),
        }
    }

    fn credential_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<JsonData>, FakeStoreError>> + Send {
        let checked = self.fail(Failure::CredentialScan);
        ready(checked.map(|()| self.credentials.clone()))
    }

    fn update_credential(
        &self,
        name: &str,
        row: &CredentialRow,
        actor: Option<&str>,
    ) -> impl Future<Output = Result<(), FakeStoreError>> + Send {
        if self.rejected.iter().any(|rejected| rejected == name) {
            return ready(Err(FakeStoreError {
                failure: Failure::CredentialUpdate,
            }));
        }
        self.record(Write::Credential {
            name: name.to_owned(),
            row: row.clone(),
            actor: actor.map(str::to_owned),
        });
        ready(Ok(()))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake invalidation sink failed")]
pub(super) struct SinkError;

pub(super) struct RecordingSink {
    pub log: Log,
    pub failed: bool,
}

impl RecordingSink {
    pub(super) fn accepting(store: &FakeStore) -> Self {
        Self {
            log: Arc::clone(&store.log),
            failed: false,
        }
    }
}

impl ModelTableInvalidation for RecordingSink {
    type Error = SinkError;

    fn model_table_changed(&self) -> impl Future<Output = Result<(), SinkError>> + Send {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Write::Publish);
        if self.failed {
            return ready(Err(SinkError));
        }
        ready(Ok(()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum McpFailure {
    Servers,
    ServerWrite,
    UserCredentials,
    UserCredentialWrite,
    UserEnvVars,
    UserEnvVarWrite,
}

#[derive(Debug, thiserror::Error)]
#[error("the fake MCP store failed on {failure:?}")]
pub(super) struct FakeMcpError {
    pub failure: McpFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UserWrite {
    pub user_id: String,
    pub server_id: String,
    pub value: String,
}

#[derive(Default)]
pub(super) struct FakeMcpStore {
    pub servers: Vec<StoredServerRow>,
    pub user_credentials: Vec<StoredUserCredentialRow>,
    pub user_env_vars: Vec<StoredUserEnvVarRow>,
    pub failures: Vec<McpFailure>,
    pub rejected_servers: Vec<String>,
    pub rejected_user_credentials: Vec<String>,
    pub rejected_user_env_vars: Vec<String>,
    pub updates: Mutex<Vec<ServerUpdate>>,
    pub credential_log: Mutex<Vec<UserWrite>>,
    pub env_var_log: Mutex<Vec<UserWrite>>,
}

impl FakeMcpStore {
    fn fail(&self, failure: McpFailure) -> Result<(), FakeMcpError> {
        if self.failures.contains(&failure) {
            return Err(FakeMcpError { failure });
        }
        Ok(())
    }

    pub(super) fn server_updates(&self) -> Vec<ServerUpdate> {
        self.updates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn user_credential_writes(&self) -> Vec<UserWrite> {
        self.credential_log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn user_env_var_writes(&self) -> Vec<UserWrite> {
        self.env_var_log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

// Both per-user stages write the re-sealed value; the fake records it and fails the users it was told to.
macro_rules! recorded_user_write {
    ($method:ident, $value:ident, $log:ident, $rejected:ident, $failure:ident $(,)?) => {
        fn $method(
            &self,
            user_id: &str,
            server_id: &str,
            $value: &str,
        ) -> impl Future<Output = Result<(), FakeMcpError>> + Send {
            if self.$rejected.iter().any(|rejected| rejected == user_id) {
                return ready(Err(FakeMcpError {
                    failure: McpFailure::$failure,
                }));
            }
            self.$log
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(UserWrite {
                    user_id: user_id.to_owned(),
                    server_id: server_id.to_owned(),
                    value: $value.to_owned(),
                });
            ready(Ok(()))
        }
    };
}

impl McpRotationStore for FakeMcpStore {
    type Error = FakeMcpError;

    fn server_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredServerRow>, FakeMcpError>> + Send {
        let checked = self.fail(McpFailure::Servers);
        ready(checked.map(|()| self.servers.clone()))
    }

    fn update_server(
        &self,
        server_id: &str,
        update: &ServerUpdate,
    ) -> impl Future<Output = Result<(), FakeMcpError>> + Send {
        if self
            .rejected_servers
            .iter()
            .any(|rejected| rejected == server_id)
        {
            return ready(Err(FakeMcpError {
                failure: McpFailure::ServerWrite,
            }));
        }
        self.updates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(update.clone());
        ready(Ok(()))
    }

    fn oauth_client_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredOAuthClientRow>, FakeMcpError>> + Send {
        ready(Ok(Vec::new()))
    }

    fn update_oauth_client(
        &self,
        _server_id: &str,
        _credentials: &JsonValue,
    ) -> impl Future<Output = Result<(), FakeMcpError>> + Send {
        ready(Ok(()))
    }

    fn user_credential_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredUserCredentialRow>, FakeMcpError>> + Send {
        let checked = self.fail(McpFailure::UserCredentials);
        ready(checked.map(|()| self.user_credentials.clone()))
    }

    recorded_user_write!(
        update_user_credential,
        credential_b64,
        credential_log,
        rejected_user_credentials,
        UserCredentialWrite,
    );

    fn user_env_var_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredUserEnvVarRow>, FakeMcpError>> + Send {
        let checked = self.fail(McpFailure::UserEnvVars);
        ready(checked.map(|()| self.user_env_vars.clone()))
    }

    recorded_user_write!(
        update_user_env_var,
        values_b64,
        env_var_log,
        rejected_user_env_vars,
        UserEnvVarWrite,
    );
}

pub(super) struct Fixture {
    pub store: FakeStore,
    pub sink: RecordingSink,
    pub mcp: FakeMcpStore,
    pub assertions: MemoryAssertionRows,
    pub cipher: Arc<DbCipher>,
    pub new_master_key: SecretString,
    pub secrets: Arc<SecretLookup>,
    pub warnings: Arc<Warnings>,
    pub tasks: TaskTracker,
    pub runtime: RuntimeEnvironment,
}

pub(super) fn fixture() -> Fixture {
    let store = FakeStore::default();
    let sink = RecordingSink::accepting(&store);
    Fixture {
        store,
        sink,
        mcp: FakeMcpStore::default(),
        assertions: MemoryAssertionRows::default(),
        cipher: Arc::new(cipher(OLD_KEY)),
        new_master_key: new_key(),
        secrets: Arc::new(secrets()),
        warnings: Arc::new(Warnings::default()),
        tasks: TaskTracker::new(),
        runtime: RuntimeEnvironment::default(),
    }
}

impl Fixture {
    pub(super) fn rotation(
        &mut self,
    ) -> Rotation<'_, FakeStore, FakeMcpStore, MemoryAssertionRows, RecordingSink> {
        Rotation {
            store: &self.store,
            invalidate: &self.sink,
            mcp: &self.mcp,
            assertions: &self.assertions,
            cipher: &self.cipher,
            new_master_key: &self.new_master_key,
            secrets: &self.secrets,
            warnings: &self.warnings,
            tasks: &self.tasks,
            runtime: &mut self.runtime,
        }
    }
}

pub(super) fn saved_models(writes: &[Write]) -> Vec<PreparedModel> {
    writes
        .iter()
        .filter_map(|write| match write {
            Write::Models(models) => Some(models.as_slice()),
            Write::Environment(_) | Write::Credential { .. } | Write::Publish => None,
        })
        .flatten()
        .cloned()
        .collect()
}

pub(super) fn saved_environment(writes: &[Write]) -> Option<JsonData> {
    writes.iter().find_map(|write| match write {
        Write::Models(_) | Write::Credential { .. } | Write::Publish => None,
        Write::Environment(values) => Some(values.clone()),
    })
}

pub(super) fn credential_writes(writes: &[Write]) -> Vec<(String, CredentialRow, Option<String>)> {
    let mut saved = Vec::new();
    for write in writes {
        match write {
            Write::Credential { name, row, actor } => {
                saved.push((name.clone(), row.clone(), actor.clone()));
            }
            Write::Models(_) | Write::Environment(_) | Write::Publish => {}
        }
    }
    saved
}

pub(super) fn published(writes: &[Write]) -> bool {
    writes.iter().any(|write| match write {
        Write::Publish => true,
        Write::Models(_) | Write::Environment(_) | Write::Credential { .. } => false,
    })
}

pub(super) fn payload_values(row: &CredentialRow) -> &BTreeMap<String, JsonData> {
    &row.fields.credential_values.0
}

pub(super) fn params_of(model: &PreparedModel) -> BTreeMap<String, JsonData> {
    let JsonData::Object(fields) =
        serde_json::from_str(&model.litellm_params).expect("the prepared params are JSON")
    else {
        panic!("the prepared params are a JSON object");
    };
    fields
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        from_utf8(&bytes)
            .expect("the tracing writer stores UTF-8")
            .to_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
