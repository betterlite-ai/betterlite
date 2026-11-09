use std::collections::BTreeMap;
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::redis_cache_values::{RedisCacheValues, RedisCacheValuesSettings};
use crate::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_connection_settings::model::settings::RedisConnectionSettings;
use crate::redis_live_client::LiveClientFactory;
use crate::redis_live_client::tests::fake_server::FakeServer;

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
    Named(&'static str),
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

    pub(crate) fn named(name: &'static str) -> Arc<Self> {
        Arc::new(Self {
            open: GateOpen::Named(name),
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
        let open = self.open == GateOpen::Always
            || matches!(self.open, GateOpen::Named(open_name) if open_name == name);
        if open {
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
    durations: Arc<Mutex<Vec<Duration>>>,
}

impl TestHooks {
    pub(crate) fn new() -> (Arc<Self>, TestLog) {
        let log = TestLog::default();
        (
            Arc::new(Self {
                log: log.clone(),
                durations: Arc::new(Mutex::new(Vec::new())),
            }),
            log,
        )
    }

    pub(crate) fn durations(&self) -> Vec<Duration> {
        match self.durations.lock() {
            Ok(durations) => durations.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn record(&self, duration: Duration) {
        match self.durations.lock() {
            Ok(mut durations) => durations.push(duration),
            Err(poisoned) => poisoned.into_inner().push(duration),
        }
    }
}

impl ServiceHooks for TestHooks {
    fn success(&self, event: &ServiceEvent) {
        self.log
            .push(format!("success:{}:{:?}", event.call_type, event.key));
        self.record(event.duration);
    }

    fn failure(&self, event: &ServiceEvent, error: &(dyn Error + Send + Sync)) {
        self.log.push(format!(
            "failure:{}:{:?}:{error}",
            event.call_type, event.key
        ));
        self.record(event.duration);
    }
}

pub(crate) struct Harness {
    pub(crate) values: RedisCacheValues,
    pub(crate) gate: TestLog,
    pub(crate) hooks: TestLog,
}

pub(crate) fn settings(
    namespace: Option<&str>,
    default_ttl: Option<i64>,
    flush_size: Option<usize>,
) -> RedisCacheValuesSettings {
    RedisCacheValuesSettings {
        namespace: namespace.map(str::to_owned),
        default_ttl,
        redis_flush_size: flush_size,
    }
}

fn connection_settings(host: &str, port: u16) -> RedisConnectionSettings {
    RedisConnectionSettings {
        host: Some(host.to_owned()),
        port: Some(i64::from(port)),
        db: None,
        username: None,
        password: None,
        url: None,
        ssl: None,
        socket_timeout: Some(1.0_f64),
        socket_connect_timeout: Some(1.0_f64),
        socket_keepalive: None,
        max_connections: None,
        health_check_interval: None,
        credential_provider: false,
        credential_provider_settings: None,
        startup_nodes: None,
        sentinel_nodes: None,
        sentinel_password: None,
        service_name: None,
        ssl_password: None,
        passthrough: BTreeMap::new(),
    }
}

fn build_transport(settings: RedisConnectionSettings) -> Arc<RedisTransport> {
    Arc::new(
        RedisTransport::new(
            settings,
            RedisCacheFamily::Standalone,
            Arc::new(LiveClientFactory::new(None)),
            Arc::new(RedisClientStore::new()),
            None,
        )
        .expect("a transport"),
    )
}

pub(crate) fn transport_to(host: &str, port: u16) -> Arc<RedisTransport> {
    build_transport(connection_settings(host, port))
}

pub(crate) fn unreachable_transport() -> Arc<RedisTransport> {
    transport_to("127.0.0.1", 1)
}

pub(crate) fn transport_with_timeout(host: &str, port: u16, timeout: f64) -> Arc<RedisTransport> {
    let mut settings = connection_settings(host, port);
    settings.socket_timeout = Some(timeout);
    build_transport(settings)
}

pub(crate) fn harness(server: &FakeServer, settings: RedisCacheValuesSettings) -> Harness {
    harness_with_gate(server, settings, TestGate::never())
}

pub(crate) fn harness_with_gate(
    server: &FakeServer,
    settings: RedisCacheValuesSettings,
    gate: Arc<TestGate>,
) -> Harness {
    build(settings, transport_to(&server.host(), server.port()), gate)
}

pub(crate) fn harness_without_server(settings: RedisCacheValuesSettings) -> Harness {
    build(settings, unreachable_transport(), TestGate::never())
}

pub(crate) fn harness_with_missing_credentials(
    server: &FakeServer,
    settings: RedisCacheValuesSettings,
) -> Harness {
    let mut connection = connection_settings(&server.host(), server.port());
    connection.credential_provider = true;
    build(settings, build_transport(connection), TestGate::never())
}

pub(crate) fn sentinel_transport() -> Arc<RedisTransport> {
    let mut connection = connection_settings("127.0.0.1", 1);
    connection.sentinel_nodes = Some(vec![("127.0.0.1".to_owned(), 1)]);
    connection.service_name = Some("missing-service".to_owned());
    build_transport(connection)
}

pub(crate) fn missing_tls_transport() -> Arc<RedisTransport> {
    let mut connection = connection_settings("127.0.0.1", 1);
    connection.ssl = Some(true);
    connection.passthrough = BTreeMap::from([(
        "ssl_ca_certs".to_owned(),
        PassthroughValue::Text("/nonexistent-ca.pem".to_owned()),
    )]);
    build_transport(connection)
}

pub(crate) fn harness_on_transport(
    settings: RedisCacheValuesSettings,
    transport: Arc<RedisTransport>,
) -> Harness {
    build(settings, transport, TestGate::never())
}

fn build(
    settings: RedisCacheValuesSettings,
    transport: Arc<RedisTransport>,
    gate: Arc<TestGate>,
) -> Harness {
    let (hooks, hook_log) = TestHooks::new();
    let gate_log = gate.log();
    let values = RedisCacheValues::new(settings, transport, hooks, gate);
    Harness {
        values,
        gate: gate_log,
        hooks: hook_log,
    }
}
