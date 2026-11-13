use std::collections::BTreeMap;
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::cache_primitives::version::RedisVersionValue;
use crate::list_operations::{RedisListOperations, RedisListOperationsSettings};
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
        self.log.push(format!("recorded:{error}"));
    }

    fn swallowed(&self, admission: &BreakerAdmission, error: &(dyn Error + Send + Sync)) {
        self.log
            .push(format!("swallowed:{}:{error}", *admission.as_ref()));
    }
}

#[derive(Clone)]
pub(crate) struct TestHooks {
    log: TestLog,
    durations: Arc<Mutex<Vec<Duration>>>,
}

impl TestHooks {
    fn new() -> Self {
        Self {
            log: TestLog::default(),
            durations: Arc::new(Mutex::new(Vec::new())),
        }
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
    pub(crate) lists: RedisListOperations,
    pub(crate) gate: TestLog,
    pub(crate) hooks: TestLog,
    pub(crate) hook_handle: TestHooks,
}

impl Harness {
    pub(crate) fn durations(&self) -> Vec<Duration> {
        self.hook_handle.durations()
    }
}

fn empty_settings() -> RedisConnectionSettings {
    RedisConnectionSettings {
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
        credential_provider_settings: None,
        startup_nodes: None,
        sentinel_nodes: None,
        sentinel_password: None,
        service_name: None,
        ssl_password: None,
        passthrough: BTreeMap::new(),
    }
}

fn connection_settings(host: &str, port: u16) -> RedisConnectionSettings {
    let mut settings = empty_settings();
    settings.host = Some(host.to_owned());
    settings.port = Some(i64::from(port));
    settings.socket_timeout = Some(1.0_f64);
    settings.socket_connect_timeout = Some(1.0_f64);
    settings
}

pub(crate) fn transport_to(host: &str, port: u16) -> Arc<RedisTransport> {
    let settings = connection_settings(host, port);
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

pub(crate) fn transport_for(server: &FakeServer) -> Arc<RedisTransport> {
    transport_to(&server.host(), server.port())
}

pub(crate) fn unreachable_transport() -> Arc<RedisTransport> {
    transport_to("127.0.0.1", 1)
}

pub(crate) fn settings(
    namespace: Option<&str>,
    redis_version: &str,
) -> RedisListOperationsSettings {
    settings_with_default(namespace, redis_version, 7)
}

pub(crate) fn settings_with_default(
    namespace: Option<&str>,
    redis_version: &str,
    default_major_version: u32,
) -> RedisListOperationsSettings {
    RedisListOperationsSettings {
        namespace: namespace.map(str::to_owned),
        redis_version: RedisVersionValue::Text(redis_version.to_owned()),
        default_major_version,
    }
}

pub(crate) fn harness(server: &FakeServer, namespace: Option<&str>, version: &str) -> Harness {
    build(
        settings(namespace, version),
        transport_for(server),
        TestGate::never(),
    )
}

pub(crate) fn harness_without_server(namespace: Option<&str>) -> Harness {
    build(
        settings(namespace, "7.0.0"),
        unreachable_transport(),
        TestGate::never(),
    )
}

pub(crate) fn build(
    settings: RedisListOperationsSettings,
    transport: Arc<RedisTransport>,
    gate: Arc<TestGate>,
) -> Harness {
    let hook_handle = TestHooks::new();
    let hook_log = hook_handle.log.clone();
    let gate_log = gate.log();
    let hooks: Arc<dyn ServiceHooks> = Arc::new(hook_handle.clone());
    let lists = RedisListOperations::new(settings, transport, hooks, gate);
    Harness {
        lists,
        gate: gate_log,
        hooks: hook_log,
        hook_handle,
    }
}
