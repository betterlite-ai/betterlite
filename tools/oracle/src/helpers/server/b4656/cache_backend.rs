use std::sync::Arc;

use betterlite_server::mcp::outbound_credentials::{
    DEFAULT_MAX_SIZE, InMemoryTokenCacheBackend, TokenCacheBackend as _, TokenCacheEntries,
    TokenKey, WallClock,
};
use serde::Deserialize;

use super::runtime;
use super::support::{ManualClock, TokenInput, duration_from_millis, result_view};
use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default = "default_max_size")]
    pub(super) max_size: usize,
    pub(super) operations: Vec<Operation>,
}

const fn default_max_size() -> usize {
    DEFAULT_MAX_SIZE
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Operation {
    Get {
        user_id: String,
        server_id: String,
        now_ms: i64,
    },
    Set {
        user_id: String,
        server_id: String,
        now_ms: i64,
        token: TokenInput,
        ttl_ms: i64,
    },
    Delete {
        user_id: String,
        server_id: String,
        now_ms: i64,
    },
}

impl Operation {
    fn key(&self) -> TokenKey {
        let (user_id, server_id) = match self {
            Self::Get {
                user_id, server_id, ..
            }
            | Self::Set {
                user_id, server_id, ..
            }
            | Self::Delete {
                user_id, server_id, ..
            } => (user_id, server_id),
        };
        TokenKey {
            user_id: user_id.clone(),
            server_id: server_id.clone(),
        }
    }

    const fn now_ms(&self) -> i64 {
        match self {
            Self::Get { now_ms, .. } | Self::Set { now_ms, .. } | Self::Delete { now_ms, .. } => {
                *now_ms
            }
        }
    }
}

async fn apply(
    cache: &InMemoryTokenCacheBackend,
    operation: &Operation,
) -> Result<serde_json::Value, Box<HelperError>> {
    let key = operation.key();
    match operation {
        Operation::Get { .. } => {
            let token = cache.get(&key).await;
            Ok(serde_json::json!({"result": result_view(Ok(token))}))
        }
        Operation::Set { token, ttl_ms, .. } => {
            let minted = token.to_token()?;
            cache
                .set(&key, &minted, duration_from_millis(*ttl_ms))
                .await;
            Ok(serde_json::json!({"result": null}))
        }
        Operation::Delete { .. } => {
            cache.delete(&key).await;
            Ok(serde_json::json!({"result": null}))
        }
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let clock = Arc::new(ManualClock::default());
    let concrete_clock: Arc<ManualClock> = Arc::clone(&clock);
    let wall_clock: Arc<dyn WallClock> = concrete_clock;
    let cache = InMemoryTokenCacheBackend {
        max_size: input.max_size,
        clock: wall_clock,
        entries: TokenCacheEntries::default(),
    };
    runtime()?.block_on(async {
        let mut results = Vec::new();
        for operation in &input.operations {
            clock.set_millis(operation.now_ms())?;
            results.push(apply(&cache, operation).await?);
        }
        Ok(serde_json::json!({"operations": results}))
    })
}
