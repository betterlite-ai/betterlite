use std::sync::Arc;

use betterlite_server::mcp::outbound_credentials::cached_store::CachedOAuthTokenStore;
use betterlite_server::mcp::outbound_credentials::{
    DEFAULT_EXPIRY_SKEW, DEFAULT_MAX_SIZE, InMemoryTokenCacheBackend,
    InvalidatableOAuthTokenStore as _, OAuthTokenStore as _, TokenCacheEntries, WallClock,
};
use serde::Deserialize;

use super::runtime;
use super::support::{ManualClock, ScriptedStore, TokenOutcome, result_view};
use crate::helpers::{HelperError, HelperResult};

type Store = CachedOAuthTokenStore<ScriptedStore, InMemoryTokenCacheBackend>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default = "default_max_size")]
    pub(super) max_size: usize,
    pub(super) inner: Vec<TokenOutcome>,
    pub(super) operations: Vec<Operation>,
}

const fn default_max_size() -> usize {
    DEFAULT_MAX_SIZE
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Operation {
    Fetch {
        user_id: String,
        server_id: String,
        now_ms: i64,
    },
    Invalidate {
        user_id: String,
        server_id: String,
        now_ms: i64,
    },
}

async fn apply(
    store: &Store,
    setter: &ManualClock,
    operation: &Operation,
) -> Result<serde_json::Value, Box<HelperError>> {
    match operation {
        Operation::Fetch {
            user_id,
            server_id,
            now_ms,
        } => {
            setter.set_millis(*now_ms)?;
            let result = store.fetch(user_id, server_id).await;
            Ok(serde_json::json!({
                "result": result_view(result),
                "inner_reads": store.inner.reads.load(std::sync::atomic::Ordering::SeqCst),
            }))
        }
        Operation::Invalidate {
            user_id,
            server_id,
            now_ms,
        } => {
            setter.set_millis(*now_ms)?;
            store.invalidate(user_id, server_id).await;
            Ok(serde_json::json!({
                "result": null,
                "inner_reads": store.inner.reads.load(std::sync::atomic::Ordering::SeqCst),
            }))
        }
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let clock = Arc::new(ManualClock::default());
    let backend_concrete: Arc<ManualClock> = Arc::clone(&clock);
    let backend_clock: Arc<dyn WallClock> = backend_concrete;
    let setter = Arc::clone(&clock);
    let cached_clock: Arc<dyn WallClock> = clock;
    let store = CachedOAuthTokenStore {
        inner: ScriptedStore::new(input.inner.clone())?,
        default_ttl: std::time::Duration::from_secs(300),
        expiry_skew: DEFAULT_EXPIRY_SKEW,
        backend: InMemoryTokenCacheBackend {
            max_size: input.max_size,
            clock: backend_clock,
            entries: TokenCacheEntries::default(),
        },
        clock: cached_clock,
    };
    runtime()?.block_on(async {
        let mut results = Vec::new();
        for operation in &input.operations {
            results.push(apply(&store, &setter, operation).await?);
        }
        Ok(serde_json::json!({"operations": results}))
    })
}
