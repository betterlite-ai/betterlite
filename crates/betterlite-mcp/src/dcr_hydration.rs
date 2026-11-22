mod tests;

use std::convert::Infallible;
use std::error::Error as StdError;
use std::future::Future;

use betterlite_db::mcp_stored_contracts::secret::StoredSecret;
use betterlite_db::mcp_stored_contracts::{JsonValue, MCPTokenEndpointAuthMethod};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use thiserror::Error;

use crate::server_record::{MCPServer, SecretCredential};

#[derive(Debug, Error)]
pub enum StoreReadError<Source>
where
    Source: StdError + 'static,
{
    #[error("the persisted DCR client could not be read: {source}")]
    Read {
        #[source]
        source: Source,
    },
}

pub trait DcrClientStore: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn client_credentials(
        &self,
        server_id: &str,
    ) -> impl Future<Output = Result<Option<JsonValue>, StoreReadError<Self::Error>>> + Send;
}

#[derive(Clone, Default, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct PersistedDcrCredentials {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub token_endpoint_auth_method: Option<String>,
    pub redirect_uris: Option<Vec<String>>,
}

#[must_use]
pub fn parse_persisted_credentials(blob: &JsonValue) -> Option<PersistedDcrCredentials> {
    match blob {
        JsonValue::Null => None,
        JsonValue::Object(fields) if fields.is_empty() => None,
        JsonValue::Object(_) => decode_credentials(blob),
        JsonValue::String(text) => parse_credentials_text(text),
        JsonValue::Array(_) | JsonValue::Bool(_) | JsonValue::Number(_) => None,
    }
}

fn parse_credentials_text(text: &str) -> Option<PersistedDcrCredentials> {
    let parsed = serde_json::from_str::<JsonValue>(text).ok()?;
    match parsed {
        JsonValue::Object(_) => serde_json::from_str(text).ok(),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => None,
    }
}

fn decode_credentials(blob: &JsonValue) -> Option<PersistedDcrCredentials> {
    let text = serde_json::to_string(blob).ok()?;
    serde_json::from_str(&text).ok()
}

#[must_use]
pub fn decrypt_credential(cipher: &DbCipher, value: Option<&str>, field: &str) -> Option<String> {
    let text = value?;
    let sealed = StoredValue::<Infallible>::Text(SecretString::from(text.to_owned()));
    let opened = cipher.decrypt_value(
        sealed,
        field,
        DecryptPolicy {
            log: FailureLog::Debug,
            value: FailureValue::Original,
        },
    )?;
    match opened {
        StoredValue::Text(secret) => Some(secret.expose_secret().to_owned()),
        StoredValue::Other(never) => match never {},
    }
}

pub struct DcrHydration<Store> {
    pub store: Store,
    pub cipher: DbCipher,
}

impl<Store> DcrHydration<Store>
where
    Store: DcrClientStore,
{
    #[must_use]
    pub fn apply_persisted_credentials(
        &self,
        server: &mut MCPServer,
        credentials: &PersistedDcrCredentials,
    ) -> bool {
        let decrypted =
            decrypt_credential(&self.cipher, credentials.client_id.as_deref(), "client_id");
        let Some(client_id) = decrypted.filter(|value| !value.is_empty()) else {
            return false;
        };
        server.fields.client_id = Some(client_id);
        server.fields.client_secret = decrypt_credential(
            &self.cipher,
            credentials.client_secret.as_deref(),
            "client_secret",
        )
        .map(|secret| SecretCredential(StoredSecret(SecretString::from(secret))));
        server.fields.token_endpoint_auth_method =
            parse_token_auth_method(credentials.token_endpoint_auth_method.as_deref());
        true
    }

    pub async fn load_store_credentials(&self, server_id: &str) -> Option<PersistedDcrCredentials> {
        let blob = match self.store.client_credentials(server_id).await {
            Ok(blob) => blob,
            Err(error) => {
                tracing::debug!(
                    "register_client_with_server: failed to read stored DCR client for server_id={server_id}: {error}"
                );
                return None;
            }
        };
        let credentials = parse_persisted_credentials(blob.as_ref()?)?;
        if credentials
            .client_id
            .as_deref()
            .is_some_and(|client_id| !client_id.is_empty())
        {
            return Some(credentials);
        }
        None
    }

    #[must_use]
    // ast-grep-ignore: no-long-names
    pub async fn hydrate_config_server_dcr_client(&self, server: &mut MCPServer) -> bool {
        if server
            .fields
            .client_id
            .as_deref()
            .is_some_and(|client_id| !client_id.is_empty())
        {
            return false;
        }
        let Some(credentials) = self.load_store_credentials(&server.fields.server_id).await else {
            return false;
        };
        self.apply_persisted_credentials(server, &credentials)
    }
}

fn parse_token_auth_method(raw: Option<&str>) -> Option<MCPTokenEndpointAuthMethod> {
    let text = raw?;
    match text.parse::<MCPTokenEndpointAuthMethod>() {
        Ok(method) => Some(method),
        Err(error) => {
            tracing::debug!(
                "the stored token_endpoint_auth_method {text} is not one betterlite knows: {error}"
            );
            None
        }
    }
}

#[must_use]
pub fn redirect_uri_not_registered(
    credentials: &PersistedDcrCredentials,
    current_redirect_uri: &str,
) -> bool {
    let Some(recorded) = credentials.redirect_uris.as_ref() else {
        return false;
    };
    if recorded.is_empty() {
        return false;
    }
    !recorded.iter().any(|uri| uri == current_redirect_uri)
}
