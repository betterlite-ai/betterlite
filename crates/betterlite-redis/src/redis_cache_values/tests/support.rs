use std::collections::BTreeMap;
use std::error::Error;
use std::sync::{Arc, Mutex};

use crate::redis_cache_values::{RedisCacheValues, RedisCacheValuesSettings};
use crate::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
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
}

impl TestHooks {
    pub(crate) fn new() -> (Arc<Self>, TestLog) {
        let log = TestLog::default();
        (Arc::new(Self { log: log.clone() }), log)
    }
}

impl ServiceHooks for TestHooks {
    fn success(&self, event: &ServiceEvent) {
        self.log
            .push(format!("success:{}:{:?}", event.call_type, event.key));
    }

    fn failure(&self, event: &ServiceEvent, error: &(dyn Error + Send + Sync)) {
        self.log
            .push(format!("failure:{}:{error}", event.call_type));
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

pub(crate) fn transport_to(host: &str, port: u16) -> Arc<RedisTransport> {
    Arc::new(
        RedisTransport::new(
            connection_settings(host, port),
            RedisCacheFamily::Standalone,
            Arc::new(LiveClientFactory::new(None)),
            Arc::new(RedisClientStore::new()),
            None,
        )
        .expect("a transport"),
    )
}

pub(crate) fn unreachable_transport() -> Arc<RedisTransport> {
    transport_to("127.0.0.1", 1)
}

fn harness_from(
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

pub(crate) fn harness(
    server: &FakeServer,
    namespace: Option<&str>,
    default_ttl: Option<i64>,
    flush_size: Option<usize>,
) -> Harness {
    harness_from(
        settings(namespace, default_ttl, flush_size),
        transport_to(&server.host(), server.port()),
        TestGate::never(),
    )
}

pub(crate) fn harness_with_gate(
    server: &FakeServer,
    namespace: Option<&str>,
    default_ttl: Option<i64>,
    flush_size: Option<usize>,
    gate: Arc<TestGate>,
) -> Harness {
    harness_from(
        settings(namespace, default_ttl, flush_size),
        transport_to(&server.host(), server.port()),
        gate,
    )
}

pub(crate) fn harness_without_server(namespace: Option<&str>, default_ttl: Option<i64>) -> Harness {
    harness_from(
        settings(namespace, default_ttl, None),
        unreachable_transport(),
        TestGate::never(),
    )
}

pub(crate) fn harness_with_open_gate(
    server: &FakeServer,
    namespace: Option<&str>,
    default_ttl: Option<i64>,
    flush_size: Option<usize>,
) -> Harness {
    harness_with_gate(
        server,
        namespace,
        default_ttl,
        flush_size,
        TestGate::always(),
    )
}
