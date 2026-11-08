use std::collections::BTreeMap;
use std::error::Error;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::runtime::{Builder, Runtime};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use crate::redis_connection_settings::model::nodes::ClusterNodeConfig;
use crate::redis_connection_settings::model::settings::RedisConnectionSettings;
use crate::redis_live_client::LiveClientFactory;
use crate::redis_live_client::tests::fake_server::FakeServer;

use crate::redis_counters::RedisCounters;

pub(crate) const DEFAULT_TTL: i64 = 60;

pub(crate) const ADMISSION: u64 = 41;

pub(crate) fn runtime() -> Runtime {
    Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a test runtime")
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
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

pub(crate) fn direct_settings(server: &FakeServer) -> RedisConnectionSettings {
    let mut settings = empty_settings();
    settings.host = Some(server.host());
    settings.port = Some(i64::from(server.port()));
    settings
}

pub(crate) fn transport(
    settings: RedisConnectionSettings,
    family: RedisCacheFamily,
) -> Arc<RedisTransport> {
    Arc::new(
        RedisTransport::new(
            settings,
            family,
            Arc::new(LiveClientFactory::new(None)),
            Arc::new(RedisClientStore::new()),
            None,
        )
        .expect("a redis transport"),
    )
}

pub(crate) fn counters(
    transport: Arc<RedisTransport>,
    namespace: Option<&str>,
) -> (RedisCounters, HookLog, GateLog) {
    let hooks = HookLog::default();
    let gate = GateLog::default();
    let counters = RedisCounters::new(
        transport,
        namespace.map(str::to_owned),
        DEFAULT_TTL,
        Arc::new(TestHooks { log: hooks.clone() }),
        Arc::new(TestGate { log: gate.clone() }),
    );
    (counters, hooks, gate)
}

pub(crate) struct ClusterPair {
    pub(crate) first: FakeServer,
    pub(crate) second: FakeServer,
    pub(crate) transport: Arc<RedisTransport>,
}

pub(crate) async fn cluster_pair() -> ClusterPair {
    let first = FakeServer::start().await;
    let second = FakeServer::start().await;
    let slots = vec![
        (0, 8191, vec![(first.host(), first.port())]),
        (8192, 16383, vec![(second.host(), second.port())]),
    ];
    first.set_cluster_slots(slots.clone());
    second.set_cluster_slots(slots);
    let mut settings = empty_settings();
    settings.startup_nodes = Some(vec![
        ClusterNodeConfig {
            host: first.host(),
            port: i64::from(first.port()),
        },
        ClusterNodeConfig {
            host: second.host(),
            port: i64::from(second.port()),
        },
    ]);
    ClusterPair {
        first,
        second,
        transport: transport(settings, RedisCacheFamily::Cluster),
    }
}

pub(crate) fn key_in_slot_range(low: u16, high: u16) -> &'static str {
    const CANDIDATES: [&str; 12] = [
        "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
        "lambda", "mu",
    ];
    CANDIDATES
        .into_iter()
        .find(|key| (low..=high).contains(&redis_slot(key)))
        .expect("a key hashing into the slot range")
}

fn redis_slot(key: &str) -> u16 {
    crc16::State::<crc16::XMODEM>::calculate(key.as_bytes()) & 0x3fff
}

#[derive(Clone, Default)]
pub(crate) struct GateLog {
    enters: Arc<Mutex<Vec<&'static str>>>,
    exits: Arc<Mutex<Vec<u64>>>,
    failures: Arc<Mutex<Vec<String>>>,
    swallowed: Arc<Mutex<Vec<String>>>,
    open: Arc<AtomicBool>,
}

impl GateLog {
    pub(crate) fn set_open(&self, open: bool) {
        self.open.store(open, Ordering::Relaxed);
    }

    pub(crate) fn enters(&self) -> Vec<&'static str> {
        locked(&self.enters).clone()
    }

    pub(crate) fn exits(&self) -> Vec<u64> {
        locked(&self.exits).clone()
    }

    pub(crate) fn failures(&self) -> Vec<String> {
        locked(&self.failures).clone()
    }
}

struct TestGate {
    log: GateLog,
}

impl BreakerGate for TestGate {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        if self.log.open.load(Ordering::Relaxed) {
            return Err(BreakerOpen { name });
        }
        locked(&self.log.enters).push(name);
        Ok(BreakerAdmission::from(ADMISSION))
    }

    fn exit(&self, admission: BreakerAdmission) {
        locked(&self.log.exits).push(*admission.as_ref());
    }

    fn record_failure(&self, error: &(dyn Error + Send + Sync)) {
        locked(&self.log.failures).push(error.to_string());
    }

    fn swallowed(&self, admission: &BreakerAdmission, error: &(dyn Error + Send + Sync)) {
        locked(&self.log.swallowed).push(format!("{}:{error}", *admission.as_ref()));
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HookEvent {
    pub(crate) call_type: &'static str,
    pub(crate) key: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) duration: Duration,
}

#[derive(Clone, Default)]
pub(crate) struct HookLog {
    successes: Arc<Mutex<Vec<HookEvent>>>,
    failures: Arc<Mutex<Vec<HookEvent>>>,
}

impl HookLog {
    pub(crate) fn successes(&self) -> Vec<HookEvent> {
        locked(&self.successes).clone()
    }

    pub(crate) fn failures(&self) -> Vec<HookEvent> {
        locked(&self.failures).clone()
    }

    pub(crate) fn success_names(&self) -> Vec<&'static str> {
        self.successes()
            .into_iter()
            .map(|event| event.call_type)
            .collect()
    }

    pub(crate) fn failure_names(&self) -> Vec<&'static str> {
        self.failures()
            .into_iter()
            .map(|event| event.call_type)
            .collect()
    }
}

struct TestHooks {
    log: HookLog,
}

impl ServiceHooks for TestHooks {
    fn success(&self, event: &ServiceEvent) {
        locked(&self.log.successes).push(HookEvent {
            call_type: event.call_type,
            key: event.key.clone(),
            error: None,
            duration: event.duration,
        });
    }

    fn failure(&self, event: &ServiceEvent, error: &(dyn Error + Send + Sync)) {
        locked(&self.log.failures).push(HookEvent {
            call_type: event.call_type,
            key: event.key.clone(),
            error: Some(error.to_string()),
            duration: event.duration,
        });
    }
}

#[derive(Clone, Default)]
pub(crate) struct LogCapture {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl LogCapture {
    pub(crate) fn text(&self) -> String {
        String::from_utf8(locked(&self.bytes).clone()).expect("utf-8 log output")
    }
}

pub(crate) struct LogWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl io::Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        locked(&self.bytes).extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogCapture {
    type Writer = LogWriter;

    fn make_writer(&'writer self) -> Self::Writer {
        LogWriter {
            bytes: Arc::clone(&self.bytes),
        }
    }
}

pub(crate) fn capture_logs() -> (LogCapture, DefaultGuard) {
    let capture = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .finish();
    let guard = set_default(subscriber);
    (capture, guard)
}
