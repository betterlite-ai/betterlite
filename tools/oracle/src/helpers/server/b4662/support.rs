use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use betterlite_server::mcp::backfills::log::{BackfillEvent, BackfillLog};
use betterlite_server::mcp::outbound_credentials::WallClock;
use betterlite_server::mcp::outbound_credentials::cache_codec::OAuthTokenCacheCodec;
use betterlite_server::mcp::outbound_credentials::client_credentials::token_post::{
    M2mTokenEndpointPost, TokenEndpointOutcome, TokenResponseBody,
};
use betterlite_server::mcp::outbound_credentials::dual_cache::AsyncCache;
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::Value;
use tokio::task::yield_now;

use betterlite_oracle_helper_api::error::HelperError;

mod runtime;

pub(super) use runtime::block_on;

const NOW_MS: i64 = 1_700_000_000_000;

#[derive(Debug, Deserialize)]
pub(super) struct ServersInput {
    pub(super) servers: Vec<ServerSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub(super) struct ServerSpec(pub(super) Value);

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

pub(super) struct FixedClock;

impl WallClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(NOW_MS).unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum OutcomeSpec {
    Success { body: String },
    Denied { status_code: u16, detail: String },
    Unreachable { detail: String },
}

fn outcome(spec: &OutcomeSpec) -> Result<TokenEndpointOutcome, Box<HelperError>> {
    match spec {
        OutcomeSpec::Success { body } => Ok(TokenEndpointOutcome::Success {
            body: serde_json::from_str::<TokenResponseBody>(body)
                .map_err(|error| value_error(error.to_string()))?,
        }),
        OutcomeSpec::Denied {
            status_code,
            detail,
        } => Ok(TokenEndpointOutcome::Denied {
            status_code: *status_code,
            detail: detail.clone(),
        }),
        OutcomeSpec::Unreachable { detail } => Ok(TokenEndpointOutcome::Unreachable {
            detail: detail.clone(),
        }),
    }
}

#[derive(Clone)]
pub(super) struct RecordedCall {
    pub(super) form: Vec<(String, String)>,
    pub(super) headers: Vec<(String, String)>,
}

pub(super) struct ScriptedPost {
    outcomes: Mutex<VecDeque<TokenEndpointOutcome>>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl ScriptedPost {
    pub(super) fn new(specs: &[OutcomeSpec]) -> Result<Self, Box<HelperError>> {
        let outcomes = specs
            .iter()
            .map(outcome)
            .collect::<Result<VecDeque<_>, _>>()?;
        Ok(Self {
            outcomes: Mutex::new(outcomes),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn post_count(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(super) fn first_call(&self) -> Option<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
    }
}

#[async_trait]
impl M2mTokenEndpointPost for ScriptedPost {
    async fn post(
        &self,
        url: &str,
        form: &[(String, SecretString)],
        headers: &[(String, SecretString)],
    ) -> TokenEndpointOutcome {
        let call = RecordedCall {
            form: pairs(form),
            headers: pairs(headers),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        // Yielding lets concurrent callers reach the per-identity lock the way they do over a real socket.
        yield_now().await;
        let outcome = self
            .outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        outcome.unwrap_or_else(|| TokenEndpointOutcome::Unreachable {
            detail: format!("no scripted outcome left for {url}"),
        })
    }
}

fn pairs(fields: &[(String, SecretString)]) -> Vec<(String, String)> {
    fields
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

pub(super) const fn cipher() -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: None,
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(super) fn codec() -> OAuthTokenCacheCodec {
    OAuthTokenCacheCodec {
        encrypt: Box::new(|text: &str| format!("sealed:{text}")),
        decrypt: Box::new(|blob: &str| blob.strip_prefix("sealed:").map(str::to_owned)),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(super) struct FakeError(pub(super) &'static str);

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(super) enum Failure {
    Get,
    Set,
    Delete,
}

pub(super) struct FakeBackend {
    pub(super) entries: Mutex<BTreeMap<String, String>>,
    sets: Mutex<Vec<(String, String, i64)>>,
    failures: BTreeSet<Failure>,
}

impl FakeBackend {
    pub(super) const fn new(failures: BTreeSet<Failure>) -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            sets: Mutex::new(Vec::new()),
            failures,
        }
    }

    pub(super) fn seed(&self, key: &str, value: &str) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value.to_owned());
    }

    pub(super) fn entry(&self, key: &str) -> Option<String> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn drop_entry(&self, key: &str) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }

    pub(super) fn recorded_ttl(&self, key: &str) -> Option<i64> {
        self.sets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(recorded, _value, _ttl)| recorded.as_str() == key)
            .map(|(_recorded, _value, ttl)| *ttl)
    }

    fn refuse(&self, failure: Failure, message: &'static str) -> Result<(), FakeError> {
        if self.failures.contains(&failure) {
            return Err(FakeError(message));
        }
        Ok(())
    }
}

#[async_trait]
impl AsyncCache for FakeBackend {
    type Error = FakeError;

    async fn async_get_cache(&self, key: &str) -> Result<Option<CacheValue>, Self::Error> {
        self.refuse(Failure::Get, "the fake cache get failed")?;
        Ok(self.entry(key).map(CacheValue::Text))
    }

    async fn async_set_cache(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), Self::Error> {
        self.refuse(Failure::Set, "the fake cache set failed")?;
        self.seed(key, value);
        self.sets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), value.to_owned(), ttl_seconds(ttl)));
        Ok(())
    }

    async fn async_delete_cache(&self, key: &str) -> Result<(), Self::Error> {
        self.refuse(Failure::Delete, "the fake cache delete failed")?;
        self.drop_entry(key);
        Ok(())
    }
}

fn ttl_seconds(ttl: Option<Duration>) -> i64 {
    ttl.map_or(0, |lifetime| {
        i64::try_from(lifetime.as_secs()).unwrap_or(i64::MAX)
    })
}

pub(super) fn authorization(headers: &[(String, String)]) -> Option<String> {
    headers
        .iter()
        .find(|(name, _value)| name.eq_ignore_ascii_case("authorization"))
        .map(|(_name, value)| value.clone())
}

pub(super) struct SilentLog;

impl BackfillLog for SilentLog {
    fn event(&self, _event: BackfillEvent<'_>) {}
}
