use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_mcp::credential_contracts::errors::CredError;
use betterlite_server::mcp::outbound_credentials::distributed_lock::{
    DistributedLock, LockAcquisition,
};
use betterlite_server::mcp::outbound_credentials::refresh_coordinator::{
    DEFAULT_KEY_PREFIX, DEFAULT_LOCK_TTL, DEFAULT_REFRESH_BUDGET, DEFAULT_WAIT_TIMEOUT,
    RedisRefreshCoordinator, RedisTunables,
};
use betterlite_server::mcp::outbound_credentials::token_endpoint::ExchangedToken;
use betterlite_server::mcp::outbound_credentials::token_endpoint::cache::ExchangedTokenCache;
use betterlite_server::mcp::outbound_credentials::token_exchange_provider::TokenExchangeTunables;
use betterlite_server::mcp::outbound_credentials::{
    RefreshCoordinator as _, RefreshFuture, RefreshStep, SystemWallClock, TokenKey,
};
use serde::Deserialize;
use serde_json::Value;

use super::support::{cred_tag, is_hex_32, runtime};
use betterlite_oracle_helper_api::helper::HelperResult;
use tokio::time;

const POLL_INTERVAL: Duration = Duration::from_millis(5);
const REFRESH_DELAY: Duration = Duration::from_millis(50);

type Coordinator = RedisRefreshCoordinator<FakeLock, String, String>;

#[derive(Debug, Deserialize)]
pub(crate) struct CacheInput {
    #[serde(default)]
    operations: Vec<Operation>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Operation {
    GetOrCompute {
        key: String,
        #[serde(default)]
        fingerprint: String,
        compute: Compute,
    },
    Invalidate {
        key: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Compute {
    Token {
        access_token: String,
        expires_in: Option<i64>,
    },
    Error {
        detail: String,
    },
}

impl Compute {
    fn future<'call>(
        &'call self,
        calls: &'call AtomicUsize,
    ) -> RefreshFuture<'call, ExchangedToken, CredError> {
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            match self {
                Self::Token {
                    access_token,
                    expires_in,
                } => Ok(ExchangedToken {
                    access_token: access_token.clone(),
                    expires_in: *expires_in,
                }),
                Self::Error { detail } => Err(CredError::UpstreamUnavailable(detail.clone())),
            }
        })
    }
}

pub(super) fn cache(input: &CacheInput) -> HelperResult {
    let cache =
        ExchangedTokenCache::new(Arc::new(SystemWallClock), TokenExchangeTunables::default());
    let calls = AtomicUsize::new(0);
    let results = runtime()?.block_on(async {
        let mut results: Vec<Value> = Vec::new();
        for operation in &input.operations {
            match operation {
                Operation::Invalidate { key } => cache.invalidate(key),
                Operation::GetOrCompute {
                    key,
                    fingerprint,
                    compute,
                } => {
                    let outcome = cache
                        .get_or_compute(key, fingerprint, compute.future(&calls))
                        .await;
                    results.push(match outcome {
                        Ok(token) => serde_json::json!({"ok": true, "token": token}),
                        Err(error) => {
                            serde_json::json!({"ok": false, "error": cred_tag(&error)})
                        }
                    });
                }
            }
        }
        results
    });
    Ok(serde_json::json!({
        "results": results,
        "compute_calls": calls.load(Ordering::SeqCst),
    }))
}

#[derive(Default)]
struct FakeLock {
    held: AtomicBool,
    record: Mutex<LockRecord>,
    releases: AtomicUsize,
    extends: AtomicUsize,
}

#[derive(Clone, Default)]
struct LockRecord {
    acquisitions: Vec<&'static str>,
    key: String,
    token: String,
}

#[async_trait]
impl DistributedLock for FakeLock {
    async fn acquire(&self, key: &str, token: &str, _ttl: Duration) -> LockAcquisition {
        let (label, outcome) = if self.held.swap(true, Ordering::SeqCst) {
            ("held", LockAcquisition::Held)
        } else {
            ("acquired", LockAcquisition::Acquired)
        };
        let mut record = self.record.lock().unwrap_or_else(PoisonError::into_inner);
        key.clone_into(&mut record.key);
        token.clone_into(&mut record.token);
        record.acquisitions.push(label);
        drop(record);
        outcome
    }

    async fn extend(&self, _key: &str, _token: &str, _ttl: Duration) -> bool {
        self.extends.fetch_add(1, Ordering::SeqCst);
        true
    }

    async fn release(&self, _key: &str, _token: &str) {
        self.releases.fetch_add(1, Ordering::SeqCst);
        self.held.store(false, Ordering::SeqCst);
    }

    async fn is_held(&self, _key: &str) -> bool {
        self.held.load(Ordering::SeqCst)
    }
}

fn refresh_step(refreshes: &AtomicUsize) -> RefreshStep<'_, String, String> {
    Box::new(move || {
        refreshes.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            time::sleep(REFRESH_DELAY).await;
            Ok("fresh".to_owned())
        })
    })
}

fn reread_step(rereads: &AtomicUsize) -> RefreshStep<'_, String, String> {
    Box::new(move || {
        rereads.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok("reread".to_owned()) })
    })
}

async fn race_two_refreshes(
    coordinator: &Coordinator,
    refreshes: &AtomicUsize,
    rereads: &AtomicUsize,
) -> Value {
    let token_key = TokenKey {
        user_id: "user".to_owned(),
        server_id: "server".to_owned(),
    };
    let first = coordinator.run(
        token_key.clone(),
        refresh_step(refreshes),
        reread_step(rereads),
    );
    let second = coordinator.run(token_key, refresh_step(refreshes), reread_step(rereads));
    let (first_result, second_result) = tokio::join!(first, second);
    let (Ok(winner), Ok(loser)) = (first_result, second_result) else {
        return serde_json::json!({"error": "coordinator returned an error"});
    };
    let record = coordinator
        .lock
        .record
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    serde_json::json!({
        "acquisitions": record.acquisitions,
        "refresh_calls": refreshes.load(Ordering::SeqCst),
        "reread_calls": rereads.load(Ordering::SeqCst),
        "release_calls": coordinator.lock.releases.load(Ordering::SeqCst),
        "extend_calls": coordinator.lock.extends.load(Ordering::SeqCst),
        "key": record.key,
        "token_is_hex_32": is_hex_32(&record.token),
        "winner": winner,
        "loser": loser,
    })
}

pub(super) fn coordinator() -> HelperResult {
    let refreshes = AtomicUsize::new(0);
    let rereads = AtomicUsize::new(0);
    let mut coordinator = Coordinator::new(FakeLock::default());
    coordinator.tunables = RedisTunables {
        key_prefix: DEFAULT_KEY_PREFIX.to_owned(),
        lock_ttl: DEFAULT_LOCK_TTL,
        refresh_budget: DEFAULT_REFRESH_BUDGET,
        wait_timeout: DEFAULT_WAIT_TIMEOUT,
        poll_interval: POLL_INTERVAL,
    };
    Ok(runtime()?.block_on(race_two_refreshes(&coordinator, &refreshes, &rereads)))
}
