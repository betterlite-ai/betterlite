use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_server::mcp::outbound_credentials::cache_codec::OAuthTokenCacheCodec;
use betterlite_server::mcp::outbound_credentials::dual_cache::{
    AsyncCache, DualCacheTokenCacheBackend,
};
use betterlite_server::mcp::outbound_credentials::{
    PER_USER_TOKEN_KEY_PREFIX, TokenCacheBackend as _, TokenKey,
};
use serde::Deserialize;
use thiserror::Error;

use super::runtime;
use super::support::{TokenInput, duration_from_millis, result_view};
use crate::helpers::{HelperError, HelperResult};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum GetBehavior {
    #[default]
    Missing,
    Stored,
    Error,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(super) cache_get: GetBehavior,
    #[serde(default)]
    pub(super) set_error: bool,
    #[serde(default)]
    pub(super) delete_error: bool,
    pub(super) operations: Vec<Operation>,
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

#[derive(Debug, Error)]
#[error("the cache is unavailable")]
struct CacheUnavailable;

struct FakeCache {
    get: GetBehavior,
    set_error: bool,
    delete_error: bool,
    gets: Mutex<Vec<String>>,
    stored: Mutex<std::collections::BTreeMap<String, String>>,
}

#[async_trait]
impl AsyncCache for FakeCache {
    type Error = CacheUnavailable;

    async fn async_get_cache(&self, key: &str) -> Result<Option<CacheValue>, CacheUnavailable> {
        self.gets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        match &self.get {
            GetBehavior::Missing => Ok(None),
            GetBehavior::Stored => Ok(self
                .stored
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(key)
                .cloned()
                .map(CacheValue::Text)),
            GetBehavior::Error => Err(CacheUnavailable),
        }
    }

    async fn async_set_cache(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), CacheUnavailable> {
        let _: Option<Duration> = ttl;
        if self.set_error {
            return Err(CacheUnavailable);
        }
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    async fn async_delete_cache(&self, key: &str) -> Result<(), CacheUnavailable> {
        if self.delete_error {
            return Err(CacheUnavailable);
        }
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
        Ok(())
    }
}

fn key(user_id: &str, server_id: &str) -> TokenKey {
    TokenKey {
        user_id: user_id.to_owned(),
        server_id: server_id.to_owned(),
    }
}

async fn apply(
    backend: &DualCacheTokenCacheBackend<FakeCache>,
    operation: &Operation,
) -> Result<serde_json::Value, Box<HelperError>> {
    match operation {
        Operation::Get {
            user_id,
            server_id,
            now_ms,
        } => {
            let _: i64 = *now_ms;
            let token = backend.get(&key(user_id, server_id)).await;
            Ok(serde_json::json!({"result": result_view(Ok(token))}))
        }
        Operation::Set {
            user_id,
            server_id,
            now_ms,
            token,
            ttl_ms,
        } => {
            let _: i64 = *now_ms;
            let minted = token.to_token()?;
            backend
                .set(
                    &key(user_id, server_id),
                    &minted,
                    duration_from_millis(*ttl_ms),
                )
                .await;
            Ok(serde_json::json!({"result": null}))
        }
        Operation::Delete {
            user_id,
            server_id,
            now_ms,
        } => {
            let _: i64 = *now_ms;
            backend.delete(&key(user_id, server_id)).await;
            Ok(serde_json::json!({"result": null}))
        }
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let backend = DualCacheTokenCacheBackend {
        cache: FakeCache {
            get: input.cache_get.clone(),
            set_error: input.set_error,
            delete_error: input.delete_error,
            gets: Mutex::new(Vec::new()),
            stored: Mutex::new(std::collections::BTreeMap::new()),
        },
        codec: OAuthTokenCacheCodec {
            encrypt: Box::new(|value: &str| format!("enc:{value}")),
            decrypt: Box::new(|blob: &str| blob.strip_prefix("enc:").map(str::to_owned)),
        },
        key_prefix: PER_USER_TOKEN_KEY_PREFIX,
    };
    runtime()?.block_on(async {
        let mut results = Vec::new();
        for operation in &input.operations {
            results.push(apply(&backend, operation).await?);
        }
        Ok(serde_json::json!({"operations": results}))
    })
}
