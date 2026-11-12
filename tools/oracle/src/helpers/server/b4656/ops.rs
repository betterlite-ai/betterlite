use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_server::mcp::outbound_credentials::cache_codec::OAuthTokenCacheCodec;
use betterlite_server::mcp::outbound_credentials::cached_store::CachedOAuthTokenStore;
use betterlite_server::mcp::outbound_credentials::dual_cache::DualCacheTokenCacheBackend;
use betterlite_server::mcp::outbound_credentials::presented_store::PresentedOAuthTokenStore;
use betterlite_server::mcp::outbound_credentials::refresh_store::{
    InProcessRefreshCoordinator, RefreshingTokenStore,
};
use betterlite_server::mcp::outbound_credentials::v2_store::V2PerUserTokenStore;
use betterlite_server::mcp::outbound_credentials::{
    BOUND_PREFIX, DEFAULT_EXPIRY_SKEW, DEFAULT_MAX_SIZE, InMemoryTokenCacheBackend, OAuthToken,
    OAuthTokenStore as _, RefreshCoordinator as _, RefreshStep, TokenCacheBackend,
    TokenCacheEntries, TokenKey, TokenStoreUnavailable,
};
use serde_json::Value;
use tokio::task::yield_now;

use super::support::{FakeCache, ManualClock, PayloadReader, ScriptedStore};
use super::{
    StoreResult, number, outcome, parse_token, result_view, runtime, text, value_error, view,
};
use crate::helpers::{HelperError, HelperResult};

type Coordinator = InProcessRefreshCoordinator<Option<OAuthToken>, TokenStoreUnavailable>;
type Step<'call> = RefreshStep<'call, Option<OAuthToken>, TokenStoreUnavailable>;

fn fake_codec() -> OAuthTokenCacheCodec {
    OAuthTokenCacheCodec {
        encrypt: Box::new(|value: &str| format!("enc:{value}")),
        decrypt: Box::new(|blob: &str| blob.strip_prefix("enc:").map(str::to_owned)),
    }
}

pub(super) fn token(input: &Value) -> HelperResult {
    let minted = parse_token(input.get("token").unwrap_or(&Value::Null))?;
    let rendered = format!("{minted:?}");
    let visible = minted
        .refresh_token
        .as_ref()
        .is_some_and(|secret| rendered.contains(secret));
    let store = PresentedOAuthTokenStore {
        token: minted.clone(),
    };
    runtime()?.block_on(async {
        let fetched = store
            .fetch("user", "server")
            .await
            .map_err(|error| value_error(error.to_string()))?;
        Ok(serde_json::json!({
            "token": view(&minted),
            "access_token_in_debug": rendered.contains(&minted.access_token),
            "refresh_token_in_debug": visible,
            "has_refresh_token_marker": rendered.contains("has_refresh_token: true"),
            "presented": fetched.as_ref().map(view),
        }))
    })
}

pub(super) fn codec(input: &Value) -> HelperResult {
    let codec = fake_codec();
    if input.get("operation").and_then(Value::as_str) != Some("encode") {
        let plaintext = text(input, "plaintext")?;
        return Ok(serde_json::json!({
            "decoded": codec.decode(&(codec.encrypt)(plaintext)).as_ref().map(view),
        }));
    }
    let minted = parse_token(input.get("token").unwrap_or(&Value::Null))?;
    let encoded = codec
        .encode(&minted)
        .map_err(|error| value_error(error.to_string()))?;
    let plaintext = (codec.decrypt)(&encoded);
    Ok(serde_json::json!({
        "bound": plaintext.is_some_and(|text| text.starts_with(BOUND_PREFIX)),
        "decoded": codec.decode(&encoded).as_ref().map(view),
    }))
}

fn refresh_step<'call>(calls: &'call AtomicUsize, result: &'call StoreResult) -> Step<'call> {
    Box::new(move || {
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            yield_now().await;
            result.clone()
        })
    })
}

fn reread_step(calls: &AtomicUsize) -> Step<'_> {
    Box::new(move || {
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        })
    })
}

async fn coordinated_call(
    flight: &Coordinator,
    key: &TokenKey,
    refreshes: &AtomicUsize,
    rereads: &AtomicUsize,
    result: &StoreResult,
) -> StoreResult {
    flight
        .run(
            key.clone(),
            refresh_step(refreshes, result),
            reread_step(rereads),
        )
        .await
}

pub(super) fn coordinator(input: &Value) -> HelperResult {
    let result = outcome(input.get("refresh").unwrap_or(&Value::Null))?;
    let flight = Coordinator::default();
    let refreshes = AtomicUsize::new(0);
    let rereads = AtomicUsize::new(0);
    let key = TokenKey {
        user_id: "user".to_owned(),
        server_id: "server".to_owned(),
    };
    runtime()?.block_on(async {
        let (first, second, third) = tokio::join!(
            coordinated_call(&flight, &key, &refreshes, &rereads, &result),
            coordinated_call(&flight, &key, &refreshes, &rereads, &result),
            coordinated_call(&flight, &key, &refreshes, &rereads, &result),
        );
        Ok(serde_json::json!({
            "results": [result_view(first), result_view(second), result_view(third)],
            "refresh_calls": refreshes.load(Ordering::SeqCst),
            "reread_calls": rereads.load(Ordering::SeqCst),
        }))
    })
}

fn duration_ms(millis: i64) -> Duration {
    if millis <= 0 {
        return Duration::ZERO;
    }
    Duration::from_millis(u64::try_from(millis).unwrap_or(u64::MAX))
}

async fn cache_step<Cache>(cache: &Cache, operation: &Value) -> Result<Value, Box<HelperError>>
where
    Cache: TokenCacheBackend,
{
    let key = TokenKey {
        user_id: text(operation, "user_id")?.to_owned(),
        server_id: text(operation, "server_id")?.to_owned(),
    };
    match operation.get("op").and_then(Value::as_str) {
        Some("set") => {
            let millis = operation
                .get("ttl_ms")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            let minted = parse_token(operation.get("token").unwrap_or(&Value::Null))?;
            cache.set(&key, &minted, duration_ms(millis)).await;
            Ok(serde_json::json!({"result": null}))
        }
        Some("delete") => {
            cache.delete(&key).await;
            Ok(serde_json::json!({"result": null}))
        }
        _ => Ok(serde_json::json!({"result": result_view(Ok(cache.get(&key).await))})),
    }
}

enum Store {
    Cache(InMemoryTokenCacheBackend),
    Dual(DualCacheTokenCacheBackend<FakeCache>),
    Cached(CachedOAuthTokenStore<ScriptedStore, InMemoryTokenCacheBackend>),
    Refreshing(RefreshingTokenStore<ScriptedStore, ScriptedStore, Coordinator>),
}

impl Store {
    async fn step(&self, operation: &Value) -> Result<Value, Box<HelperError>> {
        let user_id = text(operation, "user_id")?;
        let server_id = text(operation, "server_id")?;
        match self {
            Self::Cache(cache) => cache_step(cache, operation).await,
            Self::Dual(cache) => cache_step(cache, operation).await,
            Self::Cached(store) => Ok(serde_json::json!({
                "result": result_view(store.fetch(user_id, server_id).await),
                "inner_reads": store.inner.calls.load(Ordering::SeqCst),
            })),
            Self::Refreshing(store) => Ok(serde_json::json!({
                "result": result_view(store.fetch(user_id, server_id).await),
                "inner_reads": store.inner.calls.load(Ordering::SeqCst),
                "refresh_calls": store.refresher.calls.load(Ordering::SeqCst),
            })),
        }
    }
}

fn fake_cache(input: &Value) -> FakeCache {
    FakeCache {
        get: input
            .get("cache_get")
            .and_then(Value::as_str)
            .map(str::to_owned),
        set_error: input
            .get("set_error")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        delete_error: input
            .get("delete_error")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        stored: Mutex::new(std::collections::BTreeMap::new()),
    }
}

fn build_store(
    input: &Value,
    kind: &str,
    clock: &Arc<ManualClock>,
) -> Result<Store, Box<HelperError>> {
    let backend_clock = Arc::clone(clock);
    let store_clock = Arc::clone(clock);
    let max_size = input
        .get("max_size")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_MAX_SIZE, |value| {
            usize::try_from(value).unwrap_or(DEFAULT_MAX_SIZE)
        });
    Ok(match kind {
        "cache" => Store::Cache(InMemoryTokenCacheBackend {
            max_size,
            clock: backend_clock,
            entries: TokenCacheEntries::default(),
        }),
        "dual" => Store::Dual(DualCacheTokenCacheBackend {
            cache: fake_cache(input),
            codec: fake_codec(),
            key_prefix: "mcp:per_user_token:",
        }),
        "cached" => Store::Cached(CachedOAuthTokenStore {
            inner: ScriptedStore::new(input.get("inner"))?,
            default_ttl: Duration::from_secs(300),
            expiry_skew: DEFAULT_EXPIRY_SKEW,
            backend: InMemoryTokenCacheBackend {
                max_size,
                clock: backend_clock,
                entries: TokenCacheEntries::default(),
            },
            clock: store_clock,
        }),
        _ => Store::Refreshing(RefreshingTokenStore {
            inner: ScriptedStore::new(input.get("inner"))?,
            refresher: ScriptedStore::new(input.get("refresher"))?,
            expiry_skew: DEFAULT_EXPIRY_SKEW,
            clock: store_clock,
            coordinator: Coordinator::default(),
        }),
    })
}

pub(super) fn store(input: &Value) -> HelperResult {
    let kind = text(input, "kind")?.to_owned();
    if kind == "v2" {
        let store = V2PerUserTokenStore {
            reader: PayloadReader::new(input.get("payloads")),
        };
        let count = input
            .get("payloads")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        return runtime()?.block_on(async {
            let mut results = Vec::new();
            for _ in 0..count {
                results.push(result_view(store.fetch("user", "server").await));
            }
            Ok(serde_json::json!({"results": results}))
        });
    }
    let clock = Arc::new(ManualClock::default());
    let built = build_store(input, &kind, &clock)?;
    let operations = input
        .get("operations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    runtime()?.block_on(async {
        let mut results = Vec::new();
        for operation in &operations {
            clock.set_millis(number(operation, "now_ms")?)?;
            results.push(built.step(operation).await?);
        }
        Ok(serde_json::json!({"operations": results}))
    })
}
