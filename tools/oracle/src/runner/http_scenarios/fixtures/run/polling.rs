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
    let settings = connection_settings(polling.addr)?;
    let transport = RedisTransport::new(
        settings,
        RedisCacheFamily::Standalone,
        Arc::new(LiveClientFactory::new(None)),
        Arc::new(RedisClientStore::new()),
        None,
    )
    .map_err(|source| ScenarioError::PollingTransport { source })?;
    let cache = RedisCacheValues::new(
        RedisCacheValuesSettings {
            namespace: None,
            default_ttl: None,
            redis_flush_size: None,
        },
        Arc::new(transport),
        Arc::new(AllowHooks),
        Arc::new(AllowGate),
    );
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(ScenarioError::Io)?;
    let ttl = Duration::from_secs(ttl_seconds).as_secs_f64();
    runtime
        .block_on(cache.async_set_cache(Some(full_key), &entry, Some(ttl), false))
        .map_err(|source| ScenarioError::PollingEntry {
            key: full_key.to_owned(),
            source,
        })?;
    verify_stored(polling, full_key, value, ttl_seconds)
}

fn verify_stored(
    polling: &RedisPolling,
    full_key: &str,
    value: &str,
    ttl_seconds: u64,
) -> Result<(), ScenarioError> {
    let expected = i64::try_from(ttl_seconds).ok();
    let stored = polling.entry(full_key);
    if stored.is_some_and(|record| record.value == value && record.ttl_seconds == expected) {
        return Ok(());
    }
    Err(ScenarioError::PollingNotStored {
        key: full_key.to_owned(),
    })
}

fn connection_settings(addr: SocketAddr) -> Result<RedisConnectionSettings, ScenarioError> {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    resolve_redis_kwargs(
        RedisOverrides {
            host: Some(addr.ip().to_string()),
            port: Some(Scalar::Value(i64::from(addr.port()))),
            db: None,
            username: None,
            password: None,
            url: None,
            ssl: None,
            socket_timeout: Some(Scalar::Value(SOCKET_TIMEOUT)),
            socket_connect_timeout: Some(Scalar::Value(SOCKET_TIMEOUT)),
            socket_keepalive: None,
            max_connections: None,
            health_check_interval: None,
            credential_provider: false,
            startup_nodes: None,
            sentinel_nodes: None,
            sentinel_password: None,
            service_name: None,
            ssl_password: None,
            gcp_service_account: None,
            gcp_ssl_ca_certs: None,
            azure_redis_ad_token: None,
            azure_client_id: None,
            azure_tenant_id: None,
            azure_client_secret: None,
            aws_iam_auth: None,
            aws_iam_user_name: None,
            aws_iam_cache_name: None,
            aws_iam_region: None,
            aws_iam_serverless: None,
            passthrough: BTreeMap::new(),
        },
        ClientKind::Async,
        &environment,
    )
    .map_err(|source| ScenarioError::PollingSettings { source })
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
