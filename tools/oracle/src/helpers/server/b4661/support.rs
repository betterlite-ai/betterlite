use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_mcp::credential_contracts::auth_config::oauth::ClientCredentialsConfig;
use betterlite_mcp::credential_contracts::header_carrier::HeaderCarrier;
use betterlite_mcp::credential_contracts::token_endpoint_auth::normalize_token_auth_method;
use betterlite_server::mcp::outbound_credentials::client_credentials::token_post::{
    M2mTokenEndpointPost, TokenEndpointOutcome,
};
use betterlite_server::mcp::outbound_credentials::{
    OAuthToken, TokenCacheBackend, TokenKey, WallClock,
};
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperError;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ConfigSpec {
    #[serde(default)]
    pub(super) client_id: Option<String>,
    #[serde(default)]
    pub(super) client_secret: Option<String>,
    #[serde(default)]
    pub(super) token_url: Option<String>,
    #[serde(default)]
    pub(super) scopes: Vec<String>,
    #[serde(default)]
    pub(super) audience: Option<String>,
    #[serde(default)]
    pub(super) upstream_resource: Option<String>,
    #[serde(default)]
    pub(super) token_endpoint_auth_method: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct CarrierSpec {
    #[serde(default)]
    pub(super) header_name: Option<String>,
    #[serde(default)]
    pub(super) value_prefix: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum OutcomeSpec {
    Success { body: String },
    Denied { status_code: u16, detail: String },
    Unreachable { detail: String },
}

pub(super) fn config_from(spec: &ConfigSpec) -> Result<ClientCredentialsConfig, Box<HelperError>> {
    let token_endpoint_auth_method = spec
        .token_endpoint_auth_method
        .as_deref()
        .map(|raw| {
            normalize_token_auth_method(raw)
                .ok_or_else(|| value_error(format!("unknown token_endpoint_auth_method: {raw}")))
        })
        .transpose()?;
    Ok(ClientCredentialsConfig {
        header: HeaderCarrier::default(),
        client_id: spec.client_id.clone(),
        client_secret: spec.client_secret.clone().map(SecretString::from),
        token_url: spec.token_url.clone(),
        scopes: spec.scopes.clone(),
        audience: spec.audience.clone(),
        upstream_resource: spec.upstream_resource.clone(),
        token_endpoint_auth_method,
    })
}

pub(super) fn pairs(fields: &[(String, SecretString)]) -> Vec<[String; 2]> {
    fields
        .iter()
        .map(|(name, value)| [name.clone(), value.expose_secret().to_owned()])
        .collect()
}

pub(super) fn token_view(token: &OAuthToken) -> Value {
    let mut view = serde_json::Map::new();
    view.insert(
        "access_token".to_owned(),
        Value::from(token.access_token.as_str()),
    );
    view.insert(
        "expires_at_ms".to_owned(),
        token
            .expires_at
            .map_or(Value::Null, |at| Value::from(at.timestamp_millis())),
    );
    view.insert(
        "refresh_token".to_owned(),
        token
            .refresh_token
            .as_deref()
            .map_or(Value::Null, Value::from),
    );
    view.insert("scopes".to_owned(), json!(token.scopes));
    view.insert(
        "identity_binding_proof".to_owned(),
        token
            .identity_binding_proof
            .as_deref()
            .map_or(Value::Null, Value::from),
    );
    Value::Object(view)
}

pub(super) fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn epoch(now_ms: i64) -> Result<DateTime<Utc>, Box<HelperError>> {
    DateTime::from_timestamp_millis(now_ms)
        .ok_or_else(|| value_error(format!("invalid now_ms: {now_ms}")))
}

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| value_error(error.to_string()))
}

pub(super) struct ManualClock {
    pub(super) now: DateTime<Utc>,
}

impl WallClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

#[derive(Default)]
pub(super) struct RecordingBackend {
    entries: Mutex<HashMap<TokenKey, OAuthToken>>,
    writes: Mutex<Vec<(TokenKey, Duration)>>,
    deletes: Mutex<u64>,
}

impl RecordingBackend {
    pub(super) fn state_view(&self) -> (Value, u64) {
        let writes = self.writes.lock().unwrap_or_else(PoisonError::into_inner);
        let rows = Value::Array(
            writes
                .iter()
                .map(|(key, ttl)| {
                    json!({
                        "identity_key": key.user_id,
                        "server_id": key.server_id,
                        "ttl_ms": i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX),
                    })
                })
                .collect(),
        );
        let deletes = *self.deletes.lock().unwrap_or_else(PoisonError::into_inner);
        (rows, deletes)
    }
}

#[async_trait]
impl TokenCacheBackend for RecordingBackend {
    async fn get(&self, key: &TokenKey) -> Option<OAuthToken> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    async fn set(&self, key: &TokenKey, token: &OAuthToken, ttl: Duration) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), token.clone());
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.clone(), ttl));
    }

    async fn delete(&self, key: &TokenKey) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
        let mut deletes = self.deletes.lock().unwrap_or_else(PoisonError::into_inner);
        *deletes = deletes.saturating_add(1);
    }
}

pub(super) type RecordedCall = (String, Vec<(String, String)>);

pub(super) struct ScriptedPost {
    pub(super) outcomes: Mutex<VecDeque<TokenEndpointOutcome>>,
    pub(super) calls: Mutex<Vec<RecordedCall>>,
}

#[async_trait]
impl M2mTokenEndpointPost for ScriptedPost {
    async fn post(
        &self,
        url: &str,
        form: &[(String, SecretString)],
        _headers: &[(String, SecretString)],
    ) -> TokenEndpointOutcome {
        let recorded: Vec<(String, String)> = form
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url.to_owned(), recorded));
        let outcome = self
            .outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        outcome.unwrap_or_else(|| TokenEndpointOutcome::Unreachable {
            detail: "the case scripted no outcome".to_owned(),
        })
    }
}
