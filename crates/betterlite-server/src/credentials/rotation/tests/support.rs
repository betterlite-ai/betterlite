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
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::Rotation;
use super::sink::ModelTableInvalidation;
use super::store::RotationStore;
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

    async fn model_rows(&self) -> Result<Vec<ProxyModelTable>, FakeStoreError> {
        self.fail(Failure::ModelScan)?;
        Ok(self.models.clone())
    }

    async fn save_model_params(&self, models: &[PreparedModel]) -> Result<(), FakeStoreError> {
        self.fail(Failure::SaveModels)?;
        self.record(Write::Models(models.to_vec()));
        Ok(())
    }

    async fn config_rows(&self) -> Result<Vec<ConfigParam>, FakeStoreError> {
        self.fail(Failure::ConfigScan)?;
        Ok(self.config.clone())
    }

    async fn save_environment_variables(&self, values: JsonData) -> Result<(), FakeStoreError> {
        self.fail(Failure::SaveEnvironment)?;
        self.record(Write::Environment(values));
        Ok(())
    }

    async fn credential_rows(&self) -> Result<Vec<JsonData>, FakeStoreError> {
        self.fail(Failure::CredentialScan)?;
        Ok(self.credentials.clone())
    }

    async fn update_credential(
        &self,
        name: &str,
        row: &CredentialRow,
        actor: Option<&str>,
    ) -> Result<(), FakeStoreError> {
        if self.rejected.iter().any(|rejected| rejected == name) {
            return Err(FakeStoreError {
                failure: Failure::CredentialUpdate,
            });
        }
        self.record(Write::Credential {
            name: name.to_owned(),
            row: row.clone(),
            actor: actor.map(str::to_owned),
        });
        Ok(())
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

    async fn model_table_changed(&self) -> Result<(), SinkError> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Write::Publish);
        if self.failed {
            return Err(SinkError);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum McpFailure {
    Servers,
    UserCredentials,
    UserEnvVars,
}

#[derive(Debug, thiserror::Error)]
#[error("the fake MCP store failed on {failure:?}")]
pub(super) struct FakeMcpError {
    pub failure: McpFailure,
}

#[derive(Default)]
pub(super) struct FakeMcpStore {
    pub servers: Vec<StoredServerRow>,
    pub user_credentials: Vec<StoredUserCredentialRow>,
    pub user_env_vars: Vec<StoredUserEnvVarRow>,
    pub failures: Vec<McpFailure>,
    pub updates: Mutex<Vec<ServerUpdate>>,
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
}

impl McpRotationStore for FakeMcpStore {
    type Error = FakeMcpError;

    async fn server_rows(&self) -> Result<Vec<StoredServerRow>, FakeMcpError> {
        self.fail(McpFailure::Servers)?;
        Ok(self.servers.clone())
    }

    async fn update_server(
        &self,
        _server_id: &str,
        update: &ServerUpdate,
    ) -> Result<(), FakeMcpError> {
        self.updates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(update.clone());
        Ok(())
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

    async fn user_credential_rows(&self) -> Result<Vec<StoredUserCredentialRow>, FakeMcpError> {
        self.fail(McpFailure::UserCredentials)?;
        Ok(self.user_credentials.clone())
    }

    fn update_user_credential(
        &self,
        _user_id: &str,
        _server_id: &str,
        _credential_b64: &str,
    ) -> impl Future<Output = Result<(), FakeMcpError>> + Send {
        ready(Ok(()))
    }

    async fn user_env_var_rows(&self) -> Result<Vec<StoredUserEnvVarRow>, FakeMcpError> {
        self.fail(McpFailure::UserEnvVars)?;
        Ok(self.user_env_vars.clone())
    }

    fn update_user_env_var(
        &self,
        _user_id: &str,
        _server_id: &str,
        _values_b64: &str,
    ) -> impl Future<Output = Result<(), FakeMcpError>> + Send {
        ready(Ok(()))
    }
}

pub(super) struct Fixture {
    pub store: FakeStore,
    pub sink: RecordingSink,
    pub mcp: FakeMcpStore,
    pub assertions: MemoryAssertionRows,
    pub cipher: DbCipher,
    pub new_master_key: SecretString,
    pub secrets: SecretLookup,
    pub warnings: Warnings,
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
        cipher: cipher(OLD_KEY),
        new_master_key: new_key(),
        secrets: secrets(),
        warnings: Warnings::default(),
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
            runtime: &mut self.runtime,
        }
    }
}

pub(super) fn saved_models(writes: &[Write]) -> Vec<PreparedModel> {
    let mut saved = Vec::new();
    for write in writes {
        if let Write::Models(models) = write {
            saved.extend(models.iter().cloned());
        }
    }
    saved
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
