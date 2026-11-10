use std::collections::BTreeMap;
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::redis_cache::RedisCache;
use crate::redis_cache::connect::{RedisCacheOptions, connect};
use crate::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_connection_settings::model::scalars::{
    BoolOrText, FloatOrText, IntOrText, PassthroughValue,
};
use crate::redis_connection_settings::model::settings::RedisOverrides;
use crate::redis_live_client::LiveClientFactory;
use crate::redis_live_client::tests::fake_server::FakeServer;

pub(crate) const DEFAULT_MAJOR_VERSION: u32 = 7;
const SOCKET_TIMEOUT_SECONDS: f64 = 1.0;

#[derive(Clone, Default)]
pub(crate) struct TestLog {
    entries: Arc<Mutex<Vec<String>>>,
}

impl TestLog {
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
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum GateOpen {
    Never,
    Always,
}

pub(crate) struct TestGate {
    open: GateOpen,
    log: TestLog,
}

impl TestGate {
    pub(crate) fn never() -> Arc<Self> {
        Arc::new(Self {
            open: GateOpen::Never,
            log: TestLog::default(),
        })
    }

    pub(crate) fn always() -> Arc<Self> {
        Arc::new(Self {
            open: GateOpen::Always,
            log: TestLog::default(),
        })
    }

    pub(crate) fn log(&self) -> TestLog {
        self.log.clone()
    }
}

impl BreakerGate for TestGate {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        self.log.push(format!("enter:{name}"));
        if self.open == GateOpen::Always {
            return Err(BreakerOpen { name });
        }
        Ok(BreakerAdmission::from(1))
    }

    fn exit(&self, admission: BreakerAdmission) {
        self.log.push(format!("exit:{}", *admission.as_ref()));
    }

    fn record_failure(&self, error: &(dyn Error + Send + Sync)) {
        self.log.push(format!("failure:{error}"));
    }

    fn swallowed(&self, admission: &BreakerAdmission, error: &(dyn Error + Send + Sync)) {
        self.log
            .push(format!("swallowed:{}:{error}", *admission.as_ref()));
    }
}

pub(crate) struct TestHooks {
    log: TestLog,
    durations: Mutex<Vec<(&'static str, Duration)>>,
}

impl TestHooks {
    fn recording() -> Arc<Self> {
        Arc::new(Self {
            log: TestLog::default(),
            durations: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn entries(&self) -> Vec<String> {
        self.log.entries()
    }

    pub(crate) fn for_call(&self, call_type: &str) -> Vec<String> {
        let success = format!("success:{call_type}:");
        let failure = format!("failure:{call_type}:");
        self.entries()
            .into_iter()
            .filter(|entry| entry.starts_with(&success) || entry.starts_with(&failure))
            .collect()
    }

    pub(crate) fn durations_for(&self, call_type: &str) -> Vec<Duration> {
        let mut found = Vec::new();
        for (name, duration) in self.recorded() {
            if name == call_type {
                found.push(duration);
            }
        }
        found
    }

    fn recorded(&self) -> Vec<(&'static str, Duration)> {
        match self.durations.lock() {
            Ok(durations) => durations.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn record(&self, call_type: &'static str, duration: Duration) {
        match self.durations.lock() {
            Ok(mut durations) => durations.push((call_type, duration)),
            Err(poisoned) => poisoned.into_inner().push((call_type, duration)),
        }
    }
}

impl ServiceHooks for TestHooks {
    fn success(&self, event: &ServiceEvent) {
        self.log
            .push(format!("success:{}:{:?}", event.call_type, event.key));
        self.record(event.call_type, event.duration);
    }

    fn failure(&self, event: &ServiceEvent, error: &(dyn Error + Send + Sync)) {
        self.log.push(format!(
            "failure:{}:{:?}:{error}",
            event.call_type, event.key
        ));
        self.record(event.call_type, event.duration);
    }
}

pub(crate) struct Harness {
    pub(crate) cache: RedisCache,
    pub(crate) gate: TestLog,
    pub(crate) hooks: Arc<TestHooks>,
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

pub(crate) fn overrides(host: &str, port: u16) -> RedisOverrides {
    RedisOverrides {
        host: Some(host.to_owned()),
        port: Some(IntOrText::Value(i64::from(port))),
        socket_timeout: Some(FloatOrText::Value(SOCKET_TIMEOUT_SECONDS)),
        socket_connect_timeout: Some(FloatOrText::Value(SOCKET_TIMEOUT_SECONDS)),
        ..blank_overrides()
    }
}

pub(crate) fn overrides_without_socket_timeout(host: &str, port: u16) -> RedisOverrides {
    let mut cleared = overrides(host, port);
    cleared.socket_timeout = None;
    cleared
}

pub(crate) fn overrides_with_missing_credentials(host: &str, port: u16) -> RedisOverrides {
    let mut credential = overrides(host, port);
    credential.credential_provider = true;
    credential
}

pub(crate) fn unreadable_ca(mut overrides: RedisOverrides) -> RedisOverrides {
    overrides.ssl = Some(BoolOrText::Value(true));
    overrides.passthrough = BTreeMap::from([(
        "ssl_ca_certs".to_owned(),
        PassthroughValue::Text("/nonexistent-ca.pem".to_owned()),
    )]);
    overrides
}

pub(crate) async fn connect_cache(
    redis_overrides: RedisOverrides,
    namespace: Option<&str>,
    default_ttl: i64,
    default_major_version: u32,
    gate: Arc<TestGate>,
) -> Harness {
    let hooks = TestHooks::recording();
    let gate_log = gate.log();
    let shared_hooks = Arc::clone(&hooks);
    let service_hooks: Arc<dyn ServiceHooks> = shared_hooks;
    let shared_gate = Arc::clone(&gate);
    let breaker_gate: Arc<dyn BreakerGate> = shared_gate;
    let options = RedisCacheOptions {
        overrides: redis_overrides,
        family: RedisCacheFamily::Standalone,
        namespace: namespace.map(str::to_owned),
        default_ttl,
        redis_flush_size: None,
        default_major_version,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: service_hooks,
        gate: breaker_gate,
    };
    let env = BTreeMap::<String, String>::new();
    let cache = connect(options, &env).await.expect("a connected cache");
    Harness {
        cache,
        gate: gate_log,
        hooks,
    }
}

pub(crate) async fn harness(
    server: &FakeServer,
    namespace: Option<&str>,
    default_ttl: i64,
) -> Harness {
    harness_with_gate(server, namespace, default_ttl, TestGate::never()).await
}

pub(crate) async fn harness_with_gate(
    server: &FakeServer,
    namespace: Option<&str>,
    default_ttl: i64,
    gate: Arc<TestGate>,
) -> Harness {
    connect_cache(
        overrides(&server.host(), server.port()),
        namespace,
        default_ttl,
        DEFAULT_MAJOR_VERSION,
        gate,
    )
    .await
}

pub(crate) async fn harness_at(
    host: &str,
    port: u16,
    namespace: Option<&str>,
    default_ttl: i64,
) -> Harness {
    connect_cache(
        overrides(host, port),
        namespace,
        default_ttl,
        DEFAULT_MAJOR_VERSION,
        TestGate::never(),
    )
    .await
}
