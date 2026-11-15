use std::error::Error;
use std::future::Future;

use betterlite_db::mcp_stored_contracts::JsonValue;
use serde::Deserialize;

// A column is absent when the source column needed no change.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServerUpdate {
    pub credentials: Option<JsonValue>,
    pub env_vars: Option<JsonValue>,
    pub static_headers: Option<JsonValue>,
    pub env: Option<JsonValue>,
    pub updated_by: String,
}

impl ServerUpdate {
    #[must_use]
    pub const fn has_changes(&self) -> bool {
        self.credentials.is_some()
            || self.env_vars.is_some()
            || self.static_headers.is_some()
            || self.env.is_some()
    }
}

// The credential columns hold ciphertext, so no Debug is derived for a row.
#[derive(Clone, Default, Deserialize)]
pub struct StoredServerRow {
    pub server_id: String,
    #[serde(default)]
    pub credentials: Option<JsonValue>,
    #[serde(default)]
    pub env_vars: Option<JsonValue>,
    #[serde(default)]
    pub static_headers: Option<JsonValue>,
    #[serde(default)]
    pub env: Option<JsonValue>,
}

#[derive(Clone, Default, Deserialize)]
pub struct StoredOAuthClientRow {
    pub server_id: String,
    #[serde(default)]
    pub credentials: Option<JsonValue>,
}

#[derive(Clone, Default, Deserialize)]
pub struct StoredUserCredentialRow {
    pub user_id: String,
    pub server_id: String,
    pub credential_b64: String,
}

#[derive(Clone, Default, Deserialize)]
pub struct StoredUserEnvVarRow {
    pub user_id: String,
    pub server_id: String,
    pub values_b64: String,
}

// Each table is read with no filter, then written one changed row at a time.
pub trait McpRotationStore: Sync {
    type Error: Error + Send + 'static;

    fn server_rows(&self)
    -> impl Future<Output = Result<Vec<StoredServerRow>, Self::Error>> + Send;

    fn update_server(
        &self,
        server_id: &str,
        update: &ServerUpdate,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn oauth_client_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredOAuthClientRow>, Self::Error>> + Send;

    fn update_oauth_client(
        &self,
        server_id: &str,
        credentials: &JsonValue,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn user_credential_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredUserCredentialRow>, Self::Error>> + Send;

    fn update_user_credential(
        &self,
        user_id: &str,
        server_id: &str,
        credential_b64: &str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn user_env_var_rows(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredUserEnvVarRow>, Self::Error>> + Send;

    fn update_user_env_var(
        &self,
        user_id: &str,
        server_id: &str,
        values_b64: &str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
