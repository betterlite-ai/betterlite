pub(crate) mod cas;
pub(crate) mod codec;
pub(crate) mod typed;

use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;
use std::sync::{Arc, Mutex};

use betterlite_redis::breaker_metrics::BreakerMetrics;
use betterlite_redis::cache_primitives::namespace::check_and_fix_namespace;
use betterlite_redis::redis_client_factories::pool::build_connection_pool;
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use betterlite_redis::redis_connection_settings::model::scalars::{FloatOrText, IntOrText};
use betterlite_redis::redis_connection_settings::model::settings::{
    ClientKind, RedisConnectionSettings, RedisOverrides, Target,
};
use betterlite_redis::redis_connection_settings::resolve::resolve_redis_kwargs;
use betterlite_redis::redis_connection_settings::target::resolve_target;
use betterlite_redis::redis_counters::RedisCounters;
use betterlite_redis::redis_live_client::LiveClientFactory;
use secrecy::ExposeSecret;
use tokio::sync::Mutex as AsyncMutex;

use self::cas::CasConnection;
use super::error::{Observed, StateStoreError, StoreWarning};
use super::{BreakerClock, RedisSettings};
use crate::redis_failure_policy::run::{CallError, run_async};
use crate::redis_failure_policy::{RedisFailurePolicy, Settings as BreakerSettings};
use crate::routing_value_store::RouteKey;

pub(crate) struct RedisState {
    pub(crate) transport: Arc<RedisTransport>,
    pub(crate) counters: RedisCounters,
    pub(crate) namespace: Option<String>,
    pub(crate) default_ttl: i64,
    pub(crate) policy: Mutex<RedisFailurePolicy>,
    pub(crate) clock: BreakerClock,
    pub(crate) cas: CasConnection,
}

pub(crate) fn open(
    settings: &RedisSettings,
    breaker: BreakerSettings,
    clock: BreakerClock,
) -> Result<RedisState, StateStoreError> {
    let connection_settings = connection_settings(settings)?;
    let target = target_of(&connection_settings)?;
    let pool = build_connection_pool(&connection_settings, target, &pool_environment(settings))?;
    let factory = Arc::new(LiveClientFactory::new(None));
    let transport = Arc::new(RedisTransport::new(
        connection_settings,
        RedisCacheFamily::Standalone,
        Arc::clone(&factory),
        Arc::new(RedisClientStore::new()),
        pool,
    )?);
    let endpoint = factory
        .blocking_endpoint(&transport.async_record)
        .map_err(|source| StateStoreError::Redis {
            operation: "open",
            source,
        })?;
    let counters = RedisCounters::new(
        Arc::clone(&transport),
        None,
        settings.default_ttl,
        Arc::new(NoopHooks),
        Arc::new(NoopGate),
    );
    Ok(RedisState {
        transport,
        counters,
        namespace: settings.namespace.clone(),
        default_ttl: settings.default_ttl,
        policy: Mutex::new(RedisFailurePolicy::new(breaker, BreakerMetrics::new(None))),
        clock,
        cas: CasConnection {
            endpoint,
            slot: AsyncMutex::new(None),
        },
    })
}

fn connection_settings(
    settings: &RedisSettings,
) -> Result<RedisConnectionSettings, StateStoreError> {
    let overrides = RedisOverrides {
        host: settings.host.clone(),
        port: settings.port.map(IntOrText::Value),
        db: None,
        username: None,
        password: settings.password.clone().map(Some),
        url: settings.url.clone().map(Some),
        ssl: None,
        socket_timeout: settings.socket_timeout.map(FloatOrText::Value),
        socket_connect_timeout: None,
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
    };
    let environment: BTreeMap<String, String> = BTreeMap::new();
    Ok(resolve_redis_kwargs(
        overrides,
        ClientKind::Async,
        &environment,
    )?)
}

fn target_of(settings: &RedisConnectionSettings) -> Result<Target, StateStoreError> {
    Ok(resolve_target(
        settings.startup_nodes.as_deref(),
        settings.sentinel_nodes.as_deref(),
        settings.url.as_ref().map(ExposeSecret::expose_secret),
        settings.host.as_deref(),
    )?)
}

fn pool_environment(settings: &RedisSettings) -> BTreeMap<String, String> {
    BTreeMap::from([(
        "REDIS_CONNECTION_POOL_TIMEOUT".to_owned(),
        settings.pool_timeout_seconds.to_string(),
    )])
}

pub(crate) async fn run_observed<T, F, Fut>(
    state: &RedisState,
    operation: &'static str,
    fallback: Option<T>,
    invoke: F,
) -> Result<Observed<T>, StateStoreError>
where
    F: FnOnce() -> Fut + Send,
    Fut: Future<Output = Result<T, StateStoreError>> + Send,
{
    match run_async(&state.policy, state.clock.as_ref(), operation, |_guard| {
        invoke()
    })
    .await
    {
        Ok(outcome) => Ok(Observed {
            value: outcome.value,
            warning: outcome.warning.map(StoreWarning::Breaker),
        }),
        Err(CallError::Failed { error, warning }) => {
            let Some(value) = fallback else {
                return Err(error);
            };
            Ok(Observed {
                value,
                warning: Some(StoreWarning::Unavailable {
                    operation,
                    event: warning,
                }),
            })
        }
        Err(CallError::Open(error)) => Err(StateStoreError::CircuitOpen {
            operation: error.operation,
        }),
    }
}

pub(crate) fn namespaced(state: &RedisState, key: &RouteKey) -> String {
    check_and_fix_namespace(state.namespace.as_deref(), Some(key.as_ref()))
        .unwrap_or_else(|| key.as_ref().to_owned())
}

pub(crate) fn typed_ttl(ttl_seconds: Option<f64>) -> Result<Option<i64>, StateStoreError> {
    let Some(seconds) = ttl_seconds else {
        return Ok(None);
    };
    let Some(whole) = truncate_seconds(seconds) else {
        return Err(StateStoreError::InvalidTtl { seconds });
    };
    if whole <= 0 {
        return Err(StateStoreError::InvalidTtl { seconds });
    }
    Ok(Some(whole))
}

pub(crate) fn resolve_float_ttl(state: &RedisState, ttl_seconds: Option<f64>) -> i64 {
    ttl_seconds.map_or(state.default_ttl, |seconds| {
        truncate_seconds(seconds).unwrap_or(state.default_ttl)
    })
}

pub(crate) fn truncate_seconds(value: f64) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    value.trunc().to_string().parse().ok()
}

struct NoopHooks;

impl ServiceHooks for NoopHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

struct NoopGate;

impl BreakerGate for NoopGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}
