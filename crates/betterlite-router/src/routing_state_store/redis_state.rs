pub(crate) mod cas;
pub(crate) mod codec;
pub(crate) mod typed;

use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_redis::breaker_metrics::BreakerMetrics;
use betterlite_redis::cache_primitives::namespace::check_and_fix_namespace;
use betterlite_redis::redis_client_factories::pool::build_connection_pool;
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use betterlite_redis::redis_connection_settings::model::scalars::{
    BoolOrText, FloatOrText, IntOrText,
};
use betterlite_redis::redis_connection_settings::model::settings::{
    ClientKind, RedisConnectionSettings, RedisOverrides,
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

const DEFAULT_TTL: i64 = 60;

pub(crate) struct RedisState {
    pub(crate) transport: Arc<RedisTransport>,
    pub(crate) counters: RedisCounters,
    pub(crate) namespace: Option<String>,
    pub(crate) default_ttl: i64,
    pub(crate) update_window: Duration,
    pub(crate) policy: Mutex<RedisFailurePolicy>,
    pub(crate) clock: BreakerClock,
    pub(crate) cas: CasConnection,
}

pub(crate) fn open(
    settings: &RedisSettings,
    breaker: BreakerSettings,
    clock: BreakerClock,
) -> Result<RedisState, StateStoreError> {
    let (transport, cas) = transport(settings)?;
    let counters = RedisCounters::new(
        Arc::clone(&transport),
        None,
        DEFAULT_TTL,
        Arc::new(NoopHooks),
        Arc::new(NoopGate),
    );
    let update_window = Duration::try_from_secs_f64(settings.socket_timeout).unwrap_or_default();
    let policy = Mutex::new(RedisFailurePolicy::new(breaker, BreakerMetrics::new(None)));
    let namespace = settings.namespace.clone();
    Ok(RedisState {
        transport,
        counters,
        namespace,
        default_ttl: DEFAULT_TTL,
        update_window,
        policy,
        clock,
        cas,
    })
}

fn transport(
    settings: &RedisSettings,
) -> Result<(Arc<RedisTransport>, CasConnection), StateStoreError> {
    let connection_settings = connection_settings(settings)?;
    let target = resolve_target(
        connection_settings.startup_nodes.as_deref(),
        connection_settings.sentinel_nodes.as_deref(),
        connection_settings
            .url
            .as_ref()
            .map(ExposeSecret::expose_secret),
        connection_settings.host.as_deref(),
    )?;
    let pool_environment = BTreeMap::from([(
        "REDIS_CONNECTION_POOL_TIMEOUT".to_owned(),
        settings.pool_timeout_seconds.to_string(),
    )]);
    let pool = build_connection_pool(&connection_settings, target, &pool_environment)?;
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
    let cas = CasConnection {
        endpoint,
        slot: AsyncMutex::new(None),
    };
    Ok((transport, cas))
}

pub(crate) fn connection_settings(
    settings: &RedisSettings,
) -> Result<RedisConnectionSettings, StateStoreError> {
    let connection = &settings.connection;
    let overrides = RedisOverrides {
        host: settings.host.clone(),
        port: settings.port.map(IntOrText::Value),
        db: connection.db.map(IntOrText::Value),
        username: connection.username.clone(),
        password: settings.password.clone().map(Some),
        url: settings.url.clone().map(Some),
        ssl: connection.ssl.map(BoolOrText::Value),
        socket_timeout: Some(FloatOrText::Value(settings.socket_timeout)),
        socket_connect_timeout: connection.socket_connect_timeout.map(FloatOrText::Value),
        socket_keepalive: connection.socket_keepalive.map(BoolOrText::Value),
        max_connections: connection.max_connections.map(IntOrText::Value),
        health_check_interval: connection.health_check_interval.map(IntOrText::Value),
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
    Ok(resolve_redis_kwargs(
        overrides,
        ClientKind::Async,
        &exposed_environment(settings),
    )?)
}

fn exposed_environment(settings: &RedisSettings) -> BTreeMap<String, String> {
    settings
        .environment
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
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

pub(crate) fn typed_ttl(
    state: &RedisState,
    ttl_seconds: Option<f64>,
) -> Result<i64, StateStoreError> {
    let Some(seconds) = ttl_seconds else {
        return Ok(state.default_ttl);
    };
    truncate_seconds(seconds)
        .filter(|whole| *whole > 0)
        .ok_or(StateStoreError::InvalidTtl { seconds })
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
