#![cfg(test)]

use std::future::{Future, ready};
use std::io;
use std::str::from_utf8;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::mcp_server_encoding::credentials::CredentialBlob;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, NonceSource, StoredCipher,
    StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::super::codec::{USER_CREDENTIAL_KEY, USER_ENV_VARS_KEY, decode_user_value, sealed_text};
use super::super::store::{
    McpRotationStore, ServerUpdate, StoredOAuthClientRow, StoredServerRow, StoredUserCredentialRow,
    StoredUserEnvVarRow,
};

pub(super) const OLD_KEY: &str = "old-salt";
pub(super) const NEW_KEY: &str = "new-salt";
const UNREADABLE: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Nothing,
};
pub(super) const NONCE: [u8; 24] = [7; 24];

pub(super) struct FixedNonces;

impl NonceSource for FixedNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        let bytes = NONCE.get(..nonce.len()).ok_or(SecretError::Truncated)?;
        nonce.copy_from_slice(bytes);
        Ok(())
    }
}

pub(super) type TestCipher = StoredCipher<FixedNonces, TracingDiagnostics>;

#[must_use]
pub(super) fn cipher(key: &str) -> TestCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(key.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: FixedNonces,
        diagnostics: TracingDiagnostics,
    }
}

#[must_use]
pub(super) fn new_key() -> SecretString {
    SecretString::from(NEW_KEY)
}

#[must_use]
pub(super) fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

#[must_use]
pub(super) fn blob(text: &str) -> CredentialBlob {
    let JsonValue::Object(fields) = json(text) else {
        panic!("the expected credentials are not a JSON object");
    };
    fields
}

#[must_use]
pub(super) fn object(fields: &[(&str, JsonValue)]) -> JsonValue {
    JsonValue::Object(
        fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
}

#[must_use]
pub(super) fn string(text: &str) -> JsonValue {
    JsonValue::String(text.to_owned())
}

#[must_use]
pub(super) fn sealed(text: &str, key: &str) -> String {
    let value = StoredValue::Text(SecretString::from(text.to_owned()));
    let JsonValue::String(ciphertext) =
        sealed_text(cipher(key).encrypt_value(value, None).unwrap())
    else {
        panic!("a sealed text value is a string");
    };
    ciphertext
}

#[must_use]
pub(super) fn credential_text(text: &str, cipher: &TestCipher) -> Option<String> {
    decode_credential(text, cipher).map(|secret| secret.expose_secret().to_owned())
}

#[must_use]
pub(super) fn decode_credential(text: &str, cipher: &TestCipher) -> Option<SecretString> {
    decode_user_value(text, USER_CREDENTIAL_KEY, cipher, true)
}

#[must_use]
pub(super) fn decode_env_var(text: &str, cipher: &TestCipher) -> Option<SecretString> {
    decode_user_value(text, USER_ENV_VARS_KEY, cipher, false)
}

#[must_use]
pub(super) fn open(text: &str, key: &str) -> Option<String> {
    let value = StoredValue::<()>::Text(SecretString::from(text.to_owned()));
    match cipher(key).decrypt_value(value, "test", UNREADABLE) {
        Some(StoredValue::Text(plaintext)) => Some(plaintext.expose_secret().to_owned()),
        Some(StoredValue::Other(())) | None => None,
    }
}

#[must_use]
pub(super) fn credential_row(
    user_id: &str,
    server_id: &str,
    credential_b64: &str,
) -> StoredUserCredentialRow {
    StoredUserCredentialRow {
        user_id: user_id.to_owned(),
        server_id: server_id.to_owned(),
        credential_b64: credential_b64.to_owned(),
    }
}

#[must_use]
pub(super) fn env_row(user_id: &str, server_id: &str, values_b64: &str) -> StoredUserEnvVarRow {
    StoredUserEnvVarRow {
        user_id: user_id.to_owned(),
        server_id: server_id.to_owned(),
        values_b64: values_b64.to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Write {
    Server {
        server_id: String,
        update: ServerUpdate,
    },
    OAuthClient {
        server_id: String,
        credentials: JsonValue,
    },
    UserCredential {
        user_id: String,
        server_id: String,
        credential_b64: String,
    },
    UserEnvVar {
        user_id: String,
        server_id: String,
        values_b64: String,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("the fake store rejected write {index}")]
pub(super) struct FakeStoreError {
    pub index: usize,
}

#[derive(Default)]
pub(super) struct FakeStore {
    pub servers: Vec<StoredServerRow>,
    pub oauth_clients: Vec<StoredOAuthClientRow>,
    pub user_credentials: Vec<StoredUserCredentialRow>,
    pub user_env_vars: Vec<StoredUserEnvVarRow>,
    pub writes: Mutex<Vec<Write>>,
    pub fail_at: Option<usize>,
}

#[must_use]
pub(super) fn empty_store() -> FakeStore {
    FakeStore {
        servers: Vec::new(),
        oauth_clients: Vec::new(),
        user_credentials: Vec::new(),
        user_env_vars: Vec::new(),
        writes: Mutex::new(Vec::new()),
        fail_at: None,
    }
}

#[must_use]
pub(super) fn server_row(
    server_id: &str,
    credentials: Option<JsonValue>,
    env_vars: Option<JsonValue>,
    static_headers: Option<JsonValue>,
    env: Option<JsonValue>,
) -> StoredServerRow {
    StoredServerRow {
        server_id: server_id.to_owned(),
        credentials,
        env_vars,
        static_headers,
        env,
    }
}

#[must_use]
pub(super) fn oauth_row(server_id: &str, credentials: Option<JsonValue>) -> StoredOAuthClientRow {
    StoredOAuthClientRow {
        server_id: server_id.to_owned(),
        credentials,
    }
}

impl FakeStore {
    #[must_use]
    pub(super) fn written(&self) -> Vec<Write> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, write: Write) -> Result<(), FakeStoreError> {
        let mut writes = self.writes.lock().unwrap_or_else(PoisonError::into_inner);
        if self.fail_at == Some(writes.len()) {
            return Err(FakeStoreError {
                index: writes.len(),
            });
        }
        writes.push(write);
        drop(writes);
        Ok(())
    }
}

macro_rules! record_server_write {
    ($method:ident, $variant:ident, $parameter:ident: $ty:ty, $field:ident) => {
        fn $method(
            &self,
            server_id: &str,
            $parameter: $ty,
        ) -> impl Future<Output = Result<(), Self::Error>> + Send {
            ready(self.record(Write::$variant {
                server_id: server_id.to_owned(),
                $field: $parameter.clone(),
            }))
        }
    };
}

macro_rules! record_user_write {
    ($method:ident, $variant:ident, $value:ident) => {
        fn $method(
            &self,
            user_id: &str,
            server_id: &str,
            $value: &str,
        ) -> impl Future<Output = Result<(), Self::Error>> + Send {
            ready(self.record(Write::$variant {
                user_id: user_id.to_owned(),
                server_id: server_id.to_owned(),
                $value: $value.to_owned(),
            }))
        }
    };
}

impl McpRotationStore for FakeStore {
    type Error = FakeStoreError;

    fn server_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredServerRow>, Self::Error>> + Send {
        ready(Ok(self.servers.clone()))
    }

    record_server_write!(update_server, Server, update: &ServerUpdate, update);

    fn oauth_client_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredOAuthClientRow>, Self::Error>> + Send {
        ready(Ok(self.oauth_clients.clone()))
    }

    record_server_write!(update_oauth_client, OAuthClient, credentials: &JsonValue, credentials);

    fn user_credential_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredUserCredentialRow>, Self::Error>> + Send {
        ready(Ok(self.user_credentials.clone()))
    }

    record_user_write!(update_user_credential, UserCredential, credential_b64);

    fn user_env_var_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredUserEnvVarRow>, Self::Error>> + Send {
        ready(Ok(self.user_env_vars.clone()))
    }

    record_user_write!(update_user_env_var, UserEnvVar, values_b64);
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    #[must_use]
    pub(super) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        from_utf8(&bytes)
            .expect("the tracing writer stores UTF-8")
            .to_owned()
    }

    #[must_use]
    pub(super) fn lines(&self) -> Vec<String> {
        self.contents()
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
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

#[must_use]
pub(super) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
