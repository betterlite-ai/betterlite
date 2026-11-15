#![cfg(test)]

use std::io;
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

use super::super::codec::encrypt_stored_value;
use super::super::store::{
    McpRotationStore, ServerUpdate, StoredOAuthClientRow, StoredServerRow, StoredUserCredentialRow,
    StoredUserEnvVarRow,
};

pub const OLD_KEY: &str = "old-salt";
pub const NEW_KEY: &str = "new-salt";
pub const NONCE: [u8; 24] = [7; 24];

pub struct FixedNonces;

impl NonceSource for FixedNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        let bytes = NONCE.get(..nonce.len()).ok_or(SecretError::Truncated)?;
        nonce.copy_from_slice(bytes);
        Ok(())
    }
}

pub type TestCipher = StoredCipher<FixedNonces, TracingDiagnostics>;

#[must_use]
pub fn cipher(key: &str) -> TestCipher {
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
pub fn new_key() -> SecretString {
    SecretString::from(NEW_KEY)
}

#[must_use]
pub fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

#[must_use]
pub fn blob(text: &str) -> CredentialBlob {
    let JsonValue::Object(fields) = json(text) else {
        panic!("the expected credentials are not a JSON object");
    };
    fields
}

#[must_use]
pub fn object(fields: &[(&str, JsonValue)]) -> JsonValue {
    JsonValue::Object(
        fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
}

#[must_use]
pub fn string(text: &str) -> JsonValue {
    JsonValue::String(text.to_owned())
}

#[must_use]
pub fn sealed(text: &str, key: &str) -> String {
    match encrypt_stored_value(string(text), &cipher(key), None).unwrap() {
        JsonValue::String(ciphertext) => ciphertext,
        _other => panic!("a sealed text value is a string"),
    }
}

#[must_use]
pub fn open(text: &str, key: &str) -> Option<String> {
    let value = StoredValue::<()>::Text(SecretString::from(text.to_owned()));
    let policy = DecryptPolicy {
        log: FailureLog::Debug,
        value: FailureValue::Nothing,
    };
    match cipher(key).decrypt_value(value, "test", policy) {
        Some(StoredValue::Text(plaintext)) => Some(plaintext.expose_secret().to_owned()),
        Some(StoredValue::Other(())) | None => None,
    }
}

#[must_use]
pub fn credential_row(
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
pub fn env_row(user_id: &str, server_id: &str, values_b64: &str) -> StoredUserEnvVarRow {
    StoredUserEnvVarRow {
        user_id: user_id.to_owned(),
        server_id: server_id.to_owned(),
        values_b64: values_b64.to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Write {
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
pub struct FakeStoreError {
    pub index: usize,
}

#[derive(Default)]
pub struct FakeStore {
    pub servers: Vec<StoredServerRow>,
    pub oauth_clients: Vec<StoredOAuthClientRow>,
    pub user_credentials: Vec<StoredUserCredentialRow>,
    pub user_env_vars: Vec<StoredUserEnvVarRow>,
    pub writes: Mutex<Vec<Write>>,
    pub fail_at: Option<usize>,
}

impl FakeStore {
    #[must_use]
    pub fn written(&self) -> Vec<Write> {
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
        Ok(())
    }
}

impl McpRotationStore for FakeStore {
    type Error = FakeStoreError;

    async fn server_rows(&self) -> Result<Vec<StoredServerRow>, Self::Error> {
        Ok(self.servers.clone())
    }

    async fn update_server(
        &self,
        server_id: &str,
        update: &ServerUpdate,
    ) -> Result<(), Self::Error> {
        self.record(Write::Server {
            server_id: server_id.to_owned(),
            update: update.clone(),
        })
    }

    async fn oauth_client_rows(&self) -> Result<Vec<StoredOAuthClientRow>, Self::Error> {
        Ok(self.oauth_clients.clone())
    }

    async fn update_oauth_client(
        &self,
        server_id: &str,
        credentials: &JsonValue,
    ) -> Result<(), Self::Error> {
        self.record(Write::OAuthClient {
            server_id: server_id.to_owned(),
            credentials: credentials.clone(),
        })
    }

    async fn user_credential_rows(&self) -> Result<Vec<StoredUserCredentialRow>, Self::Error> {
        Ok(self.user_credentials.clone())
    }

    async fn update_user_credential(
        &self,
        user_id: &str,
        server_id: &str,
        credential_b64: &str,
    ) -> Result<(), Self::Error> {
        self.record(Write::UserCredential {
            user_id: user_id.to_owned(),
            server_id: server_id.to_owned(),
            credential_b64: credential_b64.to_owned(),
        })
    }

    async fn user_env_var_rows(&self) -> Result<Vec<StoredUserEnvVarRow>, Self::Error> {
        Ok(self.user_env_vars.clone())
    }

    async fn update_user_env_var(
        &self,
        user_id: &str,
        server_id: &str,
        values_b64: &str,
    ) -> Result<(), Self::Error> {
        self.record(Write::UserEnvVar {
            user_id: user_id.to_owned(),
            server_id: server_id.to_owned(),
            values_b64: values_b64.to_owned(),
        })
    }
}

#[derive(Clone, Default)]
pub struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    #[must_use]
    pub fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[must_use]
    pub fn lines(&self) -> Vec<String> {
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
pub fn capture_logs() -> (LogBuffer, DefaultGuard) {
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
