use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_server::mcp::outbound_credentials::{
    OAuthToken, OAuthTokenStore, TokenRefresher, TokenStoreUnavailable, WallClock,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::helpers::HelperError;

type StoreResult = Result<Option<OAuthToken>, TokenStoreUnavailable>;

#[derive(Debug)]
pub(super) struct ManualClock(Mutex<DateTime<Utc>>);

impl Default for ManualClock {
    fn default() -> Self {
        Self(Mutex::new(DateTime::<Utc>::UNIX_EPOCH))
    }
}

impl ManualClock {
    pub(super) fn set_millis(&self, millis: i64) -> Result<(), Box<HelperError>> {
        let instant = DateTime::from_timestamp_millis(millis)
            .ok_or_else(|| value_error(format!("invalid epoch millis: {millis}")))?;
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = instant;
        Ok(())
    }
}

impl WallClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TokenInput {
    pub(super) access_token: String,
    #[serde(default)]
    pub(super) expires_at_ms: Option<i64>,
    #[serde(default)]
    pub(super) refresh_token: Option<String>,
    #[serde(default)]
    pub(super) scopes: Vec<String>,
    #[serde(default)]
    pub(super) identity_binding_proof: Option<String>,
}

impl TokenInput {
    pub(super) fn to_token(&self) -> Result<OAuthToken, Box<HelperError>> {
        let expires_at = match self.expires_at_ms {
            Some(millis) => Some(
                DateTime::from_timestamp_millis(millis)
                    .ok_or_else(|| value_error(format!("invalid epoch millis: {millis}")))?,
            ),
            None => None,
        };
        Ok(OAuthToken {
            access_token: self.access_token.clone(),
            expires_at,
            refresh_token: self.refresh_token.clone(),
            scopes: self.scopes.clone(),
            identity_binding_proof: self.identity_binding_proof.clone(),
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum TokenOutcome {
    Token { token: TokenInput },
    Absent,
    Unavailable,
}

impl TokenOutcome {
    pub(super) fn resolve(self) -> Result<StoreResult, Box<HelperError>> {
        match self {
            Self::Token { token } => Ok(Ok(Some(token.to_token()?))),
            Self::Absent => Ok(Ok(None)),
            Self::Unavailable => Ok(Err(TokenStoreUnavailable {
                detail: "the scripted store is unavailable".to_owned(),
            })),
        }
    }
}

fn resolve_outcomes(
    outcomes: Vec<TokenOutcome>,
) -> Result<VecDeque<StoreResult>, Box<HelperError>> {
    outcomes
        .into_iter()
        .map(TokenOutcome::resolve)
        .collect::<Result<VecDeque<_>, _>>()
}

pub(super) struct ScriptedStore {
    pub(super) results: Mutex<VecDeque<StoreResult>>,
    pub(super) reads: AtomicUsize,
}

impl ScriptedStore {
    pub(super) fn new(outcomes: Vec<TokenOutcome>) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            results: Mutex::new(resolve_outcomes(outcomes)?),
            reads: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl OAuthTokenStore for ScriptedStore {
    async fn fetch(&self, _user_id: &str, _server_id: &str) -> StoreResult {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.results
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| TokenStoreUnavailable {
                detail: "the scripted store ran out of results".to_owned(),
            })?
    }
}

pub(super) struct ScriptedRefresher {
    pub(super) results: Mutex<VecDeque<StoreResult>>,
    pub(super) calls: AtomicUsize,
}

impl ScriptedRefresher {
    pub(super) fn new(outcomes: Vec<TokenOutcome>) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            results: Mutex::new(resolve_outcomes(outcomes)?),
            calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl TokenRefresher for ScriptedRefresher {
    async fn refresh(&self, _user_id: &str, _server_id: &str, _token: &OAuthToken) -> StoreResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.results
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| TokenStoreUnavailable {
                detail: "the scripted refresher ran out of results".to_owned(),
            })?
    }
}

#[derive(Serialize)]
pub(super) struct TokenView {
    access_token: String,
    expires_at_ms: Option<i64>,
    refresh_token: Option<String>,
    scopes: Vec<String>,
    identity_binding_proof: Option<String>,
}

pub(super) fn view(token: &OAuthToken) -> TokenView {
    TokenView {
        access_token: token.access_token.clone(),
        expires_at_ms: token
            .expires_at
            .map(|expires_at| expires_at.timestamp_millis()),
        refresh_token: token.refresh_token.clone(),
        scopes: token.scopes.clone(),
        identity_binding_proof: token.identity_binding_proof.clone(),
    }
}

pub(super) fn result_view(result: StoreResult) -> serde_json::Value {
    match result {
        Ok(Some(token)) => serde_json::json!(view(&token)),
        Ok(None) => serde_json::Value::Null,
        Err(_) => serde_json::json!({"error": "unavailable"}),
    }
}

pub(super) fn duration_from_millis(millis: i64) -> Duration {
    if millis <= 0 {
        return Duration::ZERO;
    }
    Duration::from_millis(u64::try_from(millis).unwrap_or(u64::MAX))
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
