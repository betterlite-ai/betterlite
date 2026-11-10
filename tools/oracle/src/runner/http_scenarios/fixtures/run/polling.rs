use std::collections::BTreeMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use betterlite_redis::redis_cache_values::{RedisCacheValues, RedisCacheValuesSettings};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use betterlite_redis::redis_connection_settings::model::scalars::Scalar;
use betterlite_redis::redis_connection_settings::model::settings::{
    ClientKind, RedisConnectionSettings, RedisOverrides,
};
use betterlite_redis::redis_connection_settings::resolve::resolve_redis_kwargs;
use betterlite_redis::redis_live_client::LiveClientFactory;
use serde_json::value::RawValue;
use tokio::runtime::Builder;

use super::redis::{Entries, StoredEntry, locked};
use crate::http_scenarios::error::ScenarioError;
use crate::http_scenarios::run::PollingTransport;

const SOCKET_TIMEOUT: f64 = 10.0;

pub(super) struct RedisPolling {
    pub(super) addr: SocketAddr,
    pub(super) entries: Entries,
}

impl PollingTransport for RedisPolling {
    fn write(
        &mut self,
        prefix: &str,
        key: &str,
        value: &str,
        ttl_seconds: u64,
    ) -> Result<(), ScenarioError> {
        store_polling(self, &format!("{prefix}:{key}"), value, ttl_seconds)
    }
}

impl RedisPolling {
    pub(super) fn entry(&self, key: &str) -> Option<StoredEntry> {
        locked(&self.entries).get(key).cloned()
    }
}

// async_set_cache swallows a connection failure, so the server's own record is the proof it stored anything.
fn store_polling(
    polling: &RedisPolling,
    full_key: &str,
    value: &str,
    ttl_seconds: u64,
) -> Result<(), ScenarioError> {
    let entry = RawValue::from_string(value.to_owned()).map_err(ScenarioError::Json)?;
    let cache = cache(polling.addr)?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(ScenarioError::Io)?;
    let ttl = Duration::from_secs(ttl_seconds).as_secs_f64();
    runtime
        .block_on(cache.async_set_cache(Some(full_key), &entry, Some(ttl), false))
        .map_err(|error| ScenarioError::Message {
            message: format!("scenario polling entry {full_key} failed: {error}"),
        })?;
    let expected = i64::try_from(ttl_seconds).ok();
    let stored = polling.entry(full_key);
    if stored.is_some_and(|record| record.value == value && record.ttl_seconds == expected) {
        return Ok(());
    }
    Err(ScenarioError::Message {
        message: "scenario polling entry was not stored with the expected value and TTL".to_owned(),
    })
}

fn cache(addr: SocketAddr) -> Result<RedisCacheValues, ScenarioError> {
    let settings = connection_settings(addr)?;
    let transport = RedisTransport::new(
        settings,
        RedisCacheFamily::Standalone,
        Arc::new(LiveClientFactory::new(None)),
        Arc::new(RedisClientStore::new()),
        None,
    )
    .map_err(|error| ScenarioError::Message {
        message: format!("scenario polling transport failed: {error}"),
    })?;
    Ok(RedisCacheValues::new(
        RedisCacheValuesSettings {
            namespace: None,
            default_ttl: None,
            redis_flush_size: None,
        },
        Arc::new(transport),
        Arc::new(AllowHooks),
        Arc::new(AllowGate),
    ))
}

fn connection_settings(addr: SocketAddr) -> Result<RedisConnectionSettings, ScenarioError> {
    let overrides = RedisOverrides {
        host: Some(addr.ip().to_string()),
        port: Some(Scalar::Value(i64::from(addr.port()))),
        socket_timeout: Some(Scalar::Value(SOCKET_TIMEOUT)),
        socket_connect_timeout: Some(Scalar::Value(SOCKET_TIMEOUT)),
        ..RedisOverrides::default()
    };
    let environment: BTreeMap<String, String> = BTreeMap::new();
    resolve_redis_kwargs(overrides, ClientKind::Async, &environment).map_err(|error| {
        ScenarioError::Message {
            message: format!("scenario polling settings failed: {error}"),
        }
    })
}

struct AllowHooks;

impl ServiceHooks for AllowHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

struct AllowGate;

impl BreakerGate for AllowGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}
