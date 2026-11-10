use std::sync::Arc;

use secrecy::ExposeSecret;
use tokio::runtime::Handle;

use super::RedisCache;
use super::error::RedisCacheError;
use crate::cache_primitives::version::{RedisVersionValue, parse_major_version};
use crate::list_operations::{RedisListOperations, RedisListOperationsSettings};
use crate::redis_cache_admin::RedisCacheAdmin;
use crate::redis_cache_admin::info_parse::InfoValue;
use crate::redis_cache_values::{RedisCacheValues, RedisCacheValuesSettings};
use crate::redis_client_factories::pool::build_connection_pool;
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use crate::redis_connection_settings::environment::EnvironmentLookup;
use crate::redis_connection_settings::model::scalars::Scalar;
use crate::redis_connection_settings::model::settings::{ClientKind, RedisOverrides};
use crate::redis_connection_settings::resolve::resolve_redis_kwargs;
use crate::redis_connection_settings::target::resolve_target;
use crate::redis_counters::RedisCounters;
use crate::redis_live_client::LiveClientFactory;

// litellm's RedisCache.__init__ defaults socket_timeout to 5.0 seconds
const SOCKET_TIMEOUT_SECONDS: f64 = 5.0;
const UNKNOWN_VERSION: &str = "Unknown";

pub struct RedisCacheOptions {
    pub overrides: RedisOverrides,
    pub family: RedisCacheFamily,
    pub namespace: Option<String>,
    pub default_ttl: i64,
    pub redis_flush_size: Option<usize>,
    pub default_major_version: u32,
    pub factory: Arc<LiveClientFactory>,
    pub store: Arc<RedisClientStore>,
    pub hooks: Arc<dyn ServiceHooks>,
    pub gate: Arc<dyn BreakerGate>,
}

pub async fn connect<E>(options: RedisCacheOptions, env: &E) -> Result<RedisCache, RedisCacheError>
where
    E: EnvironmentLookup + Sync,
{
    let transport = resolve_transport(&options, env)?;
    let (values, counters, admin) = build_slices(&transport, &options);
    let version = read_version(&admin).await;
    let redis_major_version = parse_major_version(&version, options.default_major_version);
    let lists = RedisListOperations::new(
        RedisListOperationsSettings {
            namespace: options.namespace.clone(),
            redis_version: version,
            default_major_version: options.default_major_version,
        },
        Arc::clone(&transport),
        Arc::clone(&options.hooks),
        Arc::clone(&options.gate),
    );
    admin.setup_health_pings().await;
    Ok(RedisCache {
        transport,
        values,
        counters,
        admin,
        lists,
        namespace: options.namespace,
        default_ttl: options.default_ttl,
        redis_major_version,
        hooks: options.hooks,
        gate: options.gate,
    })
}

fn resolve_transport<E>(
    options: &RedisCacheOptions,
    env: &E,
) -> Result<Arc<RedisTransport>, RedisCacheError>
where
    E: EnvironmentLookup,
{
    let overrides = with_default_socket_timeout(options.overrides.clone());
    let settings = resolve_redis_kwargs(overrides, ClientKind::Async, env)?;
    let target = resolve_target(
        settings.startup_nodes.as_deref(),
        settings.sentinel_nodes.as_deref(),
        settings.url.as_ref().map(ExposeSecret::expose_secret),
        settings.host.as_deref(),
    )?;
    let pool = build_connection_pool(&settings, target, env)?;
    let transport = RedisTransport::new(
        settings,
        options.family,
        Arc::clone(&options.factory),
        Arc::clone(&options.store),
        pool,
    )?;
    Ok(Arc::new(transport))
}

fn build_slices(
    transport: &Arc<RedisTransport>,
    options: &RedisCacheOptions,
) -> (RedisCacheValues, RedisCounters, Arc<RedisCacheAdmin>) {
    let values = RedisCacheValues::new(
        RedisCacheValuesSettings {
            namespace: options.namespace.clone(),
            default_ttl: Some(options.default_ttl),
            redis_flush_size: options.redis_flush_size,
        },
        Arc::clone(transport),
        Arc::clone(&options.hooks),
        Arc::clone(&options.gate),
    );
    let counters = RedisCounters::new(
        Arc::clone(transport),
        options.namespace.clone(),
        options.default_ttl,
        Arc::clone(&options.hooks),
        Arc::clone(&options.gate),
    );
    let admin = Arc::new(RedisCacheAdmin::new(
        Arc::clone(transport),
        options.namespace.clone(),
        Arc::clone(&options.hooks),
        Arc::clone(&options.gate),
    ));
    (values, counters, admin)
}

fn with_default_socket_timeout(mut overrides: RedisOverrides) -> RedisOverrides {
    if overrides.socket_timeout.is_none() {
        overrides.socket_timeout = Some(Scalar::Value(SOCKET_TIMEOUT_SECONDS));
    }
    overrides
}

// litellm reads redis_version through the sync client and swallows every failure, so an unreachable Redis still yields a cache
async fn read_version(admin: &Arc<RedisCacheAdmin>) -> RedisVersionValue {
    let probing = Arc::clone(admin);
    let reply = Handle::current()
        .spawn_blocking(move || probing.info())
        .await;
    let Ok(Ok(info)) = reply else {
        tracing::debug!("the redis version could not be read");
        return RedisVersionValue::Text(UNKNOWN_VERSION.to_owned());
    };
    match info.get("redis_version") {
        Some(InfoValue::Text(text)) => RedisVersionValue::Text(text.clone()),
        Some(InfoValue::Int(version)) => RedisVersionValue::Integer(*version),
        Some(InfoValue::Float(version)) => RedisVersionValue::Float(*version),
        _ => {
            tracing::debug!("the redis info reply carries no usable redis_version");
            RedisVersionValue::Text(UNKNOWN_VERSION.to_owned())
        }
    }
}
