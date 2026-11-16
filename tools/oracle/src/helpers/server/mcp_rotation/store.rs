use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, NonceSource, StoredCipher};
use betterlite_server::mcp_rotation::store::{
    McpRotationStore, ServerUpdate, StoredOAuthClientRow, StoredServerRow, StoredUserCredentialRow,
    StoredUserEnvVarRow,
};
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{Function, Input};

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
    let bytes = hex::decode(nonce).or_value_error()?;
    Ok(StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(salt.to_owned())),
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: FixedNonces(bytes),
        diagnostics: TracingDiagnostics,
    })
}

fn rows<T>(values: &[Value]) -> Result<Vec<T>, Box<HelperError>>
where
    T: DeserializeOwned,
{
    values
        .iter()
        .map(|value| serde_json::from_str(&value.to_string()).or_value_error())
        .collect()
}

pub(super) fn prepare_store(
    input: &Input,
    recorder: &Arc<Mutex<Vec<Value>>>,
) -> Result<FakeStore, Box<HelperError>> {
    let user_credentials: Vec<StoredUserCredentialRow> = match input.function {
        Function::UserCredentials => rows(&input.rows)?,
        Function::Servers | Function::UserEnvVars => Vec::new(),
    };
    let user_env_vars: Vec<StoredUserEnvVarRow> = match input.function {
        Function::UserEnvVars => rows(&input.rows)?,
        Function::Servers | Function::UserCredentials => Vec::new(),
    };
    Ok(FakeStore {
        servers: rows(&input.servers)?,
        oauth_clients: rows(&input.oauth_clients)?,
        user_credentials,
        user_env_vars,
        recorder: Arc::clone(recorder),
        fail_at: input.fail_at_write,
    })
}

#[derive(Debug, thiserror::Error)]
pub(super) enum StoreFailure {
    #[error("the injected store failure at write {index}")]
    Injected { index: usize },
    #[error("the rotated write is not JSON text")]
    Rendered(#[from] serde_json::Error),
}

// LiteLLM stores the rotated credentials and env vars as JSON text; the oracle compares structure.
fn parse_inner(value: &JsonValue) -> Result<Value, serde_json::Error> {
    let rendered = serde_json::to_value(value)?;
    if let Value::String(text) = &rendered {
        return serde_json::from_str(text);
    }
    Ok(rendered)
}

fn server_write(server_id: &str, update: &ServerUpdate) -> Result<Value, serde_json::Error> {
    let credentials = update.credentials.as_ref().map(parse_inner).transpose()?;
    let env_vars = update.env_vars.as_ref().map(parse_inner).transpose()?;
    let mut data = json!({
        "credentials": credentials,
        "env_vars": env_vars,
        "static_headers": update.static_headers,
        "env": update.env,
        "updated_by": update.updated_by,
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    Ok(json!({"table": "servers", "where": {"server_id": server_id}, "data": data}))
}

pub(super) struct FakeStore {
    pub(super) servers: Vec<StoredServerRow>,
    pub(super) oauth_clients: Vec<StoredOAuthClientRow>,
    pub(super) user_credentials: Vec<StoredUserCredentialRow>,
    pub(super) user_env_vars: Vec<StoredUserEnvVarRow>,
    pub(super) recorder: Arc<Mutex<Vec<Value>>>,
    pub(super) fail_at: Option<usize>,
}

impl FakeStore {
    fn record(&self, rendered: Result<Value, serde_json::Error>) -> Result<(), StoreFailure> {
        let write = rendered?;
        let mut writes = self.recorder.lock().unwrap_or_else(PoisonError::into_inner);
        let index = writes.len();
        if self.fail_at == Some(index) {
            return Err(StoreFailure::Injected { index });
        }
        writes.push(write);
        drop(writes);
        Ok(())
    }
}

macro_rules! user_write {
    ($method:ident, $table:literal, $field:literal, $value:ident) => {
        fn $method(
            &self,
            user_id: &str,
            server_id: &str,
            $value: &str,
        ) -> impl Future<Output = Result<(), Self::Error>> + Send {
            ready(self.record(Ok(json!({
                "table": $table,
                "where": {"user_id_server_id": {"user_id": user_id, "server_id": server_id}},
                "data": {$field: $value},
            }))))
        }
    };
}

impl McpRotationStore for FakeStore {
    type Error = StoreFailure;

    fn server_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredServerRow>, Self::Error>> + Send {
        ready(Ok(self.servers.clone()))
    }

    fn update_server(
        &self,
        server_id: &str,
        update: &ServerUpdate,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        ready(self.record(server_write(server_id, update)))
    }

    fn oauth_client_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredOAuthClientRow>, Self::Error>> + Send {
        ready(Ok(self.oauth_clients.clone()))
    }

    fn update_oauth_client(
        &self,
        server_id: &str,
        credentials: &JsonValue,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        ready(self.record(parse_inner(credentials).map(|parsed| {
            json!({
                "table": "oauth_clients",
                "where": {"server_id": server_id},
                "data": {"credentials": parsed},
            })
        })))
    }

    fn user_credential_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredUserCredentialRow>, Self::Error>> + Send {
        ready(Ok(self.user_credentials.clone()))
    }

    user_write!(
        update_user_credential,
        "user_credentials",
        "credential_b64",
        credential_b64
    );

    fn user_env_var_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredUserEnvVarRow>, Self::Error>> + Send {
        ready(Ok(self.user_env_vars.clone()))
    }

    user_write!(
        update_user_env_var,
        "user_env_vars",
        "values_b64",
        values_b64
    );
}
