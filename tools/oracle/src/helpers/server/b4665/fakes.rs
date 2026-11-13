use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_server::mcp::outbound_credentials::token_exchanger::{
    ExchangeBody, ExchangeHttpPost, ExchangeRejection,
};
use betterlite_server::mcp::outbound_credentials::{
    InMemoryTokenCacheBackend, OAuthToken, TokenCacheBackend, TokenKey, WallClock,
};
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use tokio::task::yield_now;

use super::input::{ResponseKind, ResponseSpec};
use crate::helpers::HelperError;

pub(super) struct ManualClock(pub(super) Mutex<DateTime<Utc>>);

impl ManualClock {
    pub(super) fn set(&self, now_ms: i64) -> Result<(), Box<HelperError>> {
        let instant = DateTime::from_timestamp_millis(now_ms)
            .ok_or_else(|| std::io::Error::other("now_ms is outside the supported range"))?;
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = instant;
        Ok(())
    }
}

impl WallClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(super) struct RecordingCache {
    pub(super) inner: InMemoryTokenCacheBackend,
    pub(super) events: Arc<Mutex<Vec<Value>>>,
}

#[async_trait]
impl TokenCacheBackend for RecordingCache {
    async fn get(&self, key: &TokenKey) -> Option<OAuthToken> {
        let token = self.inner.get(key).await;
        let hit = token.is_some();
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "op": "get",
                "key": key.user_id,
                "server": key.server_id,
                "hit": hit,
            }));
        token
    }

    async fn set(&self, key: &TokenKey, token: &OAuthToken, ttl: Duration) {
        let ttl_ms = i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX);
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "op": "set",
                "key": key.user_id,
                "server": key.server_id,
                "ttl_ms": ttl_ms,
            }));
        self.inner.set(key, token, ttl).await;
    }

    async fn delete(&self, key: &TokenKey) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "op": "delete",
                "key": key.user_id,
                "server": key.server_id,
            }));
        self.inner.delete(key).await;
    }
}

pub(super) struct PostState {
    pub(super) responses: VecDeque<ResponseSpec>,
    pub(super) calls: Vec<Value>,
}

pub(super) struct ScriptedPost {
    pub(super) state: Mutex<PostState>,
}

fn expose(form: &BTreeMap<String, SecretString>) -> BTreeMap<String, String> {
    form.iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

#[async_trait]
impl ExchangeHttpPost for ScriptedPost {
    async fn post(
        &self,
        url: &str,
        form: &BTreeMap<String, SecretString>,
        headers: &BTreeMap<String, SecretString>,
    ) -> Result<Option<ExchangeBody>, ExchangeRejection> {
        let recorded = json!({
            "url": url,
            "form": expose(form),
            "headers": expose(headers),
        });
        let scripted = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.calls.push(recorded);
            state.responses.pop_front()
        };
        yield_now().await;
        let Some(outcome) = scripted else {
            return Ok(None);
        };
        match outcome.kind {
            ResponseKind::Body => Ok(Some(outcome.body.unwrap_or_default())),
            ResponseKind::Transport => Ok(None),
            ResponseKind::SubjectRejected => Err(ExchangeRejection::SubjectRejected {
                detail: outcome.detail.unwrap_or_default(),
                claims: outcome.claims,
            }),
            ResponseKind::ClientFault => Err(ExchangeRejection::ClientFault {
                code: "invalid_client".to_owned(),
            }),
        }
    }
}
