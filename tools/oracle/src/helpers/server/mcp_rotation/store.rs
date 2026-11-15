use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, NonceSource, StoredCipher};
use betterlite_server::mcp_rotation::store::{
    McpRotationStore, ServerUpdate, StoredOAuthClientRow, StoredServerRow, StoredUserCredentialRow,
    StoredUserEnvVarRow,
};
use secrecy::SecretString;
use serde_json::{Value, json};

use super::helper_error;
use super::input::{CredentialInput, EnvVarInput, OAuthInput, ServerInput};
use crate::helpers::HelperError;

pub(super) type HelperCipher = StoredCipher<FixedNonces, TracingDiagnostics>;

pub(super) struct FixedNonces(Vec<u8>);

impl NonceSource for FixedNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        let bytes = self.0.get(..nonce.len()).ok_or(SecretError::Truncated)?;
        nonce.copy_from_slice(bytes);
        Ok(())
    }
}

pub(super) fn cipher(salt: &str, nonce: &str) -> Result<HelperCipher, Box<HelperError>> {
    let bytes =
        hex::decode(nonce).map_err(|source| helper_error("InvalidNonce", &source.to_string()))?;
    Ok(StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(salt.to_owned())),
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: FixedNonces(bytes),
        diagnostics: TracingDiagnostics,
    })
}

fn column(value: Option<&Value>) -> Result<Option<String>, Box<HelperError>> {
    value
        .map(serde_json::to_string)
        .transpose()
        .map_err(|source| helper_error("SerializationError", &source.to_string()))
}

pub(super) fn server_rows(
    servers: &[ServerInput],
) -> Result<Vec<StoredServerRow>, Box<HelperError>> {
    servers
        .iter()
        .map(|server| {
            Ok(StoredServerRow {
                server_id: server.server_id.clone(),
                credentials: column(server.credentials.as_ref())?,
                env_vars: column(server.env_vars.as_ref())?,
                static_headers: column(server.static_headers.as_ref())?,
                env: column(server.env.as_ref())?,
            })
        })
        .collect()
}

pub(super) fn oauth_rows(
    clients: &[OAuthInput],
) -> Result<Vec<StoredOAuthClientRow>, Box<HelperError>> {
    clients
        .iter()
        .map(|client| {
            Ok(StoredOAuthClientRow {
                server_id: client.server_id.clone(),
                credentials: column(client.credentials.as_ref())?,
            })
        })
        .collect()
}

pub(super) fn user_credential_rows(rows: &[CredentialInput]) -> Vec<StoredUserCredentialRow> {
    rows.iter()
        .map(|row| StoredUserCredentialRow {
            user_id: row.user_id.clone(),
            server_id: row.server_id.clone(),
            credential_b64: row.credential_b64.clone(),
        })
        .collect()
}

pub(super) fn user_env_rows(rows: &[EnvVarInput]) -> Vec<StoredUserEnvVarRow> {
    rows.iter()
        .map(|row| StoredUserEnvVarRow {
            user_id: row.user_id.clone(),
            server_id: row.server_id.clone(),
            values_b64: row.values_b64.clone(),
        })
        .collect()
}

fn parse_inner(value: &JsonValue) -> Result<Value, Box<HelperError>> {
    let JsonValue::String(text) = value else {
        return Err(helper_error(
            "SerializationError",
            "the rotated value is not JSON text",
        ));
    };
    serde_json::from_str(text)
        .map_err(|source| helper_error("SerializationError", &source.to_string()))
}

fn server_write(server_id: &str, update: &ServerUpdate) -> Result<Value, Box<HelperError>> {
    let mut data = serde_json::Map::new();
    if let Some(credentials) = &update.credentials {
        data.insert("credentials".to_owned(), parse_inner(credentials)?);
    }
    if let Some(env_vars) = &update.env_vars {
        data.insert("env_vars".to_owned(), parse_inner(env_vars)?);
    }
    if let Some(static_headers) = &update.static_headers {
        data.insert(
            "static_headers".to_owned(),
            serde_json::to_value(static_headers)
                .map_err(|source| helper_error("SerializationError", &source.to_string()))?,
        );
    }
    if let Some(env) = &update.env {
        data.insert(
            "env".to_owned(),
            serde_json::to_value(env)
                .map_err(|source| helper_error("SerializationError", &source.to_string()))?,
        );
    }
    data.insert(
        "updated_by".to_owned(),
        Value::String(update.updated_by.clone()),
    );
    Ok(json!({
        "table": "servers",
        "where": {"server_id": server_id},
        "data": Value::Object(data),
    }))
}

fn oauth_write(server_id: &str, credentials: &JsonValue) -> Result<Value, Box<HelperError>> {
    let credentials = parse_inner(credentials)?;
    Ok(json!({
        "table": "oauth_clients",
        "where": {"server_id": server_id},
        "data": {"credentials": credentials},
    }))
}

#[derive(Debug, thiserror::Error)]
pub(super) enum StoreFailure {
    #[error("the injected store failure at write {index}")]
    Injected { index: usize },
    #[error("the helper could not render a write: {message}")]
    Render { message: String },
}

#[derive(Default)]
pub(super) struct FakeStore {
    pub(super) servers: Vec<StoredServerRow>,
    pub(super) oauth_clients: Vec<StoredOAuthClientRow>,
    pub(super) user_credentials: Vec<StoredUserCredentialRow>,
    pub(super) user_env_vars: Vec<StoredUserEnvVarRow>,
    pub(super) recorder: Arc<Mutex<Vec<Value>>>,
    pub(super) fail_at: Option<usize>,
}

impl FakeStore {
    fn record(&self, write: Result<Value, Box<HelperError>>) -> Result<(), StoreFailure> {
        let write = write.map_err(|error| StoreFailure::Render {
            message: error.message,
        })?;
        let mut writes = self.recorder.lock().unwrap_or_else(PoisonError::into_inner);
        let index = writes.len();
        if self.fail_at == Some(index) {
            return Err(StoreFailure::Injected { index });
        }
        writes.push(write);
        Ok(())
    }
}

impl McpRotationStore for FakeStore {
    type Error = StoreFailure;

    async fn server_rows(&self) -> Result<Vec<StoredServerRow>, Self::Error> {
        Ok(self.servers.clone())
    }

    async fn update_server(
        &self,
        server_id: &str,
        update: &ServerUpdate,
    ) -> Result<(), Self::Error> {
        self.record(server_write(server_id, update))
    }

    async fn oauth_client_rows(&self) -> Result<Vec<StoredOAuthClientRow>, Self::Error> {
        Ok(self.oauth_clients.clone())
    }

    async fn update_oauth_client(
        &self,
        server_id: &str,
        credentials: &JsonValue,
    ) -> Result<(), Self::Error> {
        self.record(oauth_write(server_id, credentials))
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
        self.record(Ok(json!({
            "table": "user_credentials",
            "where": {"user_id_server_id": {"user_id": user_id, "server_id": server_id}},
            "data": {"credential_b64": credential_b64},
        })))
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
        self.record(Ok(json!({
            "table": "user_env_vars",
            "where": {"user_id_server_id": {"user_id": user_id, "server_id": server_id}},
            "data": {"values_b64": values_b64},
        })))
    }
}

pub(super) fn written(recorder: &Arc<Mutex<Vec<Value>>>) -> Vec<Value> {
    recorder
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}
