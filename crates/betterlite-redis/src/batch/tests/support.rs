use std::collections::BTreeMap;
use std::error::Error;
use std::sync::{Arc, Mutex};

use crate::redis_cache::RedisCache;
use crate::redis_cache::connect::{RedisCacheOptions, connect};
use crate::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_connection_settings::model::nodes::ClusterNodeInput;
use crate::redis_connection_settings::model::scalars::{FloatOrText, IntOrText, Scalar};
use crate::redis_connection_settings::model::settings::RedisOverrides;
use crate::redis_live_client::LiveClientFactory;
use crate::redis_live_client::tests::fake_server::{ClusterSlots, FakeServer};

pub(crate) const DEFAULT_TTL: i64 = 60;
const SOCKET_TIMEOUT_SECONDS: f64 = 1.0;

#[derive(Clone, Default)]
pub(crate) struct Log {
    entries: Arc<Mutex<Vec<String>>>,
}

impl Log {
    fn push(&self, entry: String) {
        match self.entries.lock() {
            Ok(mut entries) => entries.push(entry),
            Err(poisoned) => poisoned.into_inner().push(entry),
        }
    }

    pub(crate) fn entries(&self) -> Vec<String> {
        match self.entries.lock() {
            Ok(entries) => entries.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub(crate) fn for_call(&self, call_type: &str) -> Vec<String> {
        let success = format!("success:{call_type}");
        let failure = format!("failure:{call_type}");
        self.entries()
            .into_iter()
            .filter(|entry| entry.starts_with(&success) || entry.starts_with(&failure))
            .collect()
    }
}

struct TestGate {
    log: Log,
}

impl BreakerGate for TestGate {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        self.log.push(format!("enter:{name}"));
        Ok(BreakerAdmission::from(1))
    }

    fn exit(&self, admission: BreakerAdmission) {
        self.log.push(format!("exit:{}", *admission.as_ref()));
    }

    fn record_failure(&self, error: &(dyn Error + Send + Sync)) {
        self.log.push(format!("failure:{error}"));
    }

    fn swallowed(&self, _admission: &BreakerAdmission, error: &(dyn Error + Send + Sync)) {
        self.log.push(format!("swallowed:{error}"));
    }
}

struct TestHooks {
    log: Log,
}

impl ServiceHooks for TestHooks {
    fn success(&self, event: &ServiceEvent) {
        self.log.push(format!("success:{}", event.call_type));
    }

    fn failure(&self, event: &ServiceEvent, error: &(dyn Error + Send + Sync)) {
        self.log
            .push(format!("failure:{}:{error}", event.call_type));
    }
}

pub(crate) struct Harness {
    pub(crate) cache: Arc<RedisCache>,
    pub(crate) hooks: Log,
    pub(crate) gate: Log,
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

fn direct_overrides(host: &str, port: u16) -> RedisOverrides {
    RedisOverrides {
        host: Some(host.to_owned()),
        port: Some(IntOrText::Value(i64::from(port))),
        socket_timeout: Some(FloatOrText::Value(SOCKET_TIMEOUT_SECONDS)),
        socket_connect_timeout: Some(FloatOrText::Value(SOCKET_TIMEOUT_SECONDS)),
        ..blank_overrides()
    }
}

fn topology(server: &FakeServer) -> ClusterSlots {
    vec![(0, 16383, vec![(server.host(), server.port())])]
}

pub(crate) async fn harness(server: &FakeServer, namespace: Option<&str>) -> Harness {
    connect_harness(
        direct_overrides(&server.host(), server.port()),
        RedisCacheFamily::Standalone,
        namespace,
    )
    .await
}

pub(crate) async fn cluster_harness(server: &FakeServer, namespace: Option<&str>) -> Harness {
    server.set_cluster_slots(topology(server));
    let node = ClusterNodeInput {
        host: server.host(),
        port: IntOrText::Value(i64::from(server.port())),
    };
    let overrides = RedisOverrides {
        socket_timeout: Some(FloatOrText::Value(SOCKET_TIMEOUT_SECONDS)),
        socket_connect_timeout: Some(FloatOrText::Value(SOCKET_TIMEOUT_SECONDS)),
        startup_nodes: Some(Scalar::Value(vec![node])),
        ..blank_overrides()
    };
    connect_harness(overrides, RedisCacheFamily::Cluster, namespace).await
}

pub(crate) async fn unreachable_harness() -> Harness {
    connect_harness(
        direct_overrides("127.0.0.1", 1),
        RedisCacheFamily::Standalone,
        Some("ns"),
    )
    .await
}

async fn connect_harness(
    overrides: RedisOverrides,
    family: RedisCacheFamily,
    namespace: Option<&str>,
) -> Harness {
    let hooks = Log::default();
    let gate = Log::default();
    let service_hooks: Arc<dyn ServiceHooks> = Arc::new(TestHooks { log: hooks.clone() });
    let breaker_gate: Arc<dyn BreakerGate> = Arc::new(TestGate { log: gate.clone() });
    let options = RedisCacheOptions {
        overrides,
        family,
        namespace: namespace.map(str::to_owned),
        default_ttl: DEFAULT_TTL,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: service_hooks,
        gate: breaker_gate,
    };
    let env = BTreeMap::<String, String>::new();
    let cache = connect(options, &env).await.expect("a connected cache");
    Harness {
        cache: Arc::new(cache),
        hooks,
        gate,
    }
}
