use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::task::yield_now;

use crate::redis_cache::RedisCache;
use crate::redis_cache::connect::{RedisCacheOptions, connect};
use crate::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_connection_settings::model::scalars::{FloatOrText, IntOrText};
use crate::redis_connection_settings::model::settings::RedisOverrides;
use crate::redis_live_client::LiveClientFactory;
use crate::redis_live_client::tests::fake_server::FakeServer;

const BATCH_COMMANDS: [&str; 7] = [
    "DEL",
    "EVALSHA",
    "EXPIRE",
    "INCRBYFLOAT",
    "MGET",
    "SCRIPT",
    "SET",
];
const DEFAULT_TTL: i64 = 60;
const SOCKET_TIMEOUT_SECONDS: f64 = 1.0;

struct SilentHooks;

impl ServiceHooks for SilentHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

struct AllowGate;

impl BreakerGate for AllowGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(1))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

pub(crate) struct CountingGate {
    in_flight: AtomicUsize,
    peak: AtomicUsize,
}

impl CountingGate {
    pub(crate) fn new() -> Self {
        Self {
            in_flight: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    pub(crate) fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }

    pub(crate) fn reset_peak(&self) {
        self.peak.store(0, Ordering::SeqCst);
    }

    fn finished(&self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

impl BreakerGate for CountingGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        let in_flight = self
            .in_flight
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        self.peak.fetch_max(in_flight, Ordering::SeqCst);
        Ok(BreakerAdmission::from(1))
    }

    fn exit(&self, _admission: BreakerAdmission) {
        self.finished();
    }

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {
        self.finished();
    }

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

fn blank_overrides() -> RedisOverrides {
    RedisOverrides {
        host: None,
        port: None,
        db: None,
        username: None,
        password: None,
        url: None,
        ssl: None,
        socket_timeout: None,
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
    }
}

pub(crate) async fn cache(server: &FakeServer, namespace: Option<&str>) -> Arc<RedisCache> {
    cache_with_timeout(server, namespace, SOCKET_TIMEOUT_SECONDS).await
}

pub(crate) async fn cache_with_timeout(
    server: &FakeServer,
    namespace: Option<&str>,
    timeout_seconds: f64,
) -> Arc<RedisCache> {
    cache_with_gate(server, namespace, timeout_seconds, Arc::new(AllowGate)).await
}

pub(crate) async fn cache_with_gate<G>(
    server: &FakeServer,
    namespace: Option<&str>,
    timeout_seconds: f64,
    gate: Arc<G>,
) -> Arc<RedisCache>
where
    G: BreakerGate + 'static,
{
    let mut overrides = blank_overrides();
    overrides.host = Some(server.host());
    overrides.port = Some(IntOrText::Value(i64::from(server.port())));
    overrides.socket_timeout = Some(FloatOrText::Value(timeout_seconds));
    overrides.socket_connect_timeout = Some(FloatOrText::Value(timeout_seconds));
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: namespace.map(str::to_owned),
        default_ttl: DEFAULT_TTL,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(SilentHooks),
        gate,
    };
    let env = BTreeMap::<String, String>::new();
    Arc::new(connect(options, &env).await.expect("a connected cache"))
}

pub(crate) fn command_names(server: &FakeServer) -> Vec<String> {
    server
        .commands()
        .into_iter()
        .map(|command| command.name)
        .filter(|name| BATCH_COMMANDS.contains(&name.as_str()))
        .collect()
}

pub(crate) async fn wait_for_commands(server: &FakeServer, expected: usize) {
    let mut passes = 0_u32;
    while command_names(server).len() < expected && passes < 256_u32 {
        yield_now().await;
        passes = passes.saturating_add(1);
    }
}
