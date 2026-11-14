use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_mcp::credential_contracts::auth_config::oauth::ClientCredentialsConfig;
use betterlite_mcp::credential_contracts::header_carrier::HeaderCarrier;
use betterlite_mcp::credential_contracts::token_endpoint_auth::normalize_token_auth_method;
use betterlite_server::mcp::outbound_credentials::WallClock;
use betterlite_server::mcp::outbound_credentials::client_credentials::token_post::{
    M2mTokenEndpointPost, TokenEndpointOutcome,
};
use chrono::{DateTime, TimeDelta, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::helpers::HelperError;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ConfigSpec {
    pub(super) client_id: Option<String>,
    pub(super) client_secret: Option<String>,
    pub(super) token_url: Option<String>,
    pub(super) scopes: Vec<String>,
    pub(super) audience: Option<String>,
    pub(super) upstream_resource: Option<String>,
    pub(super) token_endpoint_auth_method: Option<String>,
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
    pub(super) now: Mutex<DateTime<Utc>>,
}

impl ManualClock {
    pub(super) fn advance(&self, millis: i64) {
        let delta = TimeDelta::try_milliseconds(millis).unwrap_or_default();
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now
            .checked_add_signed(delta)
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
    }
}

impl WallClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
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
        let mut outcomes = self.outcomes.lock().unwrap_or_else(PoisonError::into_inner);
        outcomes
            .pop_front()
            .unwrap_or_else(|| TokenEndpointOutcome::Unreachable {
                detail: "the case scripted no outcome".to_owned(),
            })
    }
}
