use std::collections::BTreeMap;
use std::error::Error;
use std::future::{Future, ready};
use std::io;
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::time::sleep;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::cache_primitives::protocol::{AsyncRedisCommands, RedisScalar};
use crate::redis_cache_admin::RedisCacheAdmin;
use crate::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use crate::redis_connection_settings::model::settings::RedisConnectionSettings;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};
use crate::redis_live_client::{LiveClientError, LiveClientFactory};

pub(crate) mod cluster;

#[derive(Debug, Clone)]
pub(crate) struct Recorded {
    pub(crate) call_type: &'static str,
    pub(crate) duration: Duration,
    pub(crate) key: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedFailure {
    pub(crate) event: Recorded,
    pub(crate) error: String,
}

#[derive(Default)]
struct HookLog {
    successes: Vec<Recorded>,
    failures: Vec<RecordedFailure>,
}

#[derive(Clone, Default)]
pub(crate) struct Hooks {
    log: Arc<Mutex<HookLog>>,
}

impl Hooks {
    pub(crate) fn successes(&self) -> Vec<Recorded> {
        self.log.lock().expect("the hook log").successes.clone()
    }

    pub(crate) fn failures(&self) -> Vec<RecordedFailure> {
        self.log.lock().expect("the hook log").failures.clone()
    }
}

impl ServiceHooks for Hooks {
    fn success(&self, event: &ServiceEvent) {
        self.log
            .lock()
            .expect("the hook log")
            .successes
            .push(record(event));
    }

    fn failure(&self, event: &ServiceEvent, error: &(dyn Error + Send + Sync)) {
        self.log
            .lock()
            .expect("the hook log")
            .failures
            .push(RecordedFailure {
                event: record(event),
                error: error.to_string(),
            });
    }
}

fn record(event: &ServiceEvent) -> Recorded {
    Recorded {
        call_type: event.call_type,
        duration: event.duration,
        key: event.key.clone(),
    }
}

#[derive(Default)]
struct GateLog {
    enters: Vec<&'static str>,
    exits: Vec<u64>,
    failures: Vec<String>,
}

#[derive(Clone, Default)]
pub(crate) struct Gate {
    open: Arc<Mutex<Option<&'static str>>>,
    log: Arc<Mutex<GateLog>>,
}

impl Gate {
    pub(crate) fn open_for(&self, name: &'static str) {
        *self.open.lock().expect("the open breaker") = Some(name);
    }

    pub(crate) fn enters(&self) -> Vec<&'static str> {
        self.state().enters.clone()
    }

    pub(crate) fn exits(&self) -> Vec<u64> {
        self.state().exits.clone()
    }

    pub(crate) fn failures(&self) -> Vec<String> {
        self.state().failures.clone()
    }

    fn state(&self) -> MutexGuard<'_, GateLog> {
        match self.log.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn breaker(&self) -> MutexGuard<'_, Option<&'static str>> {
        match self.open.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl BreakerGate for Gate {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        self.state().enters.push(name);
        if *self.breaker() == Some(name) {
            return Err(BreakerOpen { name });
        }
        Ok(BreakerAdmission::from(11_u64))
    }

    fn exit(&self, admission: BreakerAdmission) {
        self.state().exits.push(*admission.as_ref());
    }

    fn record_failure(&self, error: &(dyn Error + Send + Sync)) {
        self.state().failures.push(error.to_string());
    }

    fn swallowed(&self, admission: &BreakerAdmission, error: &(dyn Error + Send + Sync)) {
        self.state()
            .failures
            .push(format!("{}:{error}", *admission.as_ref()));
    }
}

pub(crate) struct Probe {
    pub(crate) admin: Arc<RedisCacheAdmin>,
    pub(crate) hooks: Hooks,
    pub(crate) gate: Gate,
}

pub(crate) fn probe(
    settings: &RedisConnectionSettings,
    family: RedisCacheFamily,
    namespace: Option<&str>,
) -> Probe {
    let hooks = Hooks::default();
    let gate = Gate::default();
    let store = Arc::new(RedisClientStore::new());
    let factory = Arc::new(LiveClientFactory::new(None));
    let transport = Arc::new(
        RedisTransport::new(
            settings.clone(),
            family,
            Arc::clone(&factory),
            Arc::clone(&store),
            None,
        )
        .expect("a transport"),
    );
    let admin = Arc::new(RedisCacheAdmin::new(
        transport,
        namespace.map(str::to_owned),
        Arc::new(hooks.clone()),
        Arc::new(gate.clone()),
    ));
    Probe { admin, hooks, gate }
}

pub(crate) fn empty_settings() -> RedisConnectionSettings {
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

pub(crate) fn direct_settings(host: &str, port: u16) -> RedisConnectionSettings {
    let mut settings = empty_settings();
    settings.host = Some(host.to_owned());
    settings.port = Some(i64::from(port));
    settings
}

pub(crate) enum ScriptedAnswer {
    Answer(bool),
    Failure(String),
}

pub(crate) struct ScriptedPing {
    answer: ScriptedAnswer,
    calls: AtomicUsize,
}

impl ScriptedPing {
    pub(crate) fn answering(answer: bool) -> Self {
        Self {
            answer: ScriptedAnswer::Answer(answer),
            calls: AtomicUsize::new(0),
        }
    }

    pub(crate) fn failing(message: &str) -> Self {
        Self {
            answer: ScriptedAnswer::Failure(message.to_owned()),
            calls: AtomicUsize::new(0),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    fn reject(&self) -> LiveClientError {
        match &self.answer {
            ScriptedAnswer::Answer(_) => scripted_error("the scripted client only answers ping"),
            ScriptedAnswer::Failure(message) => scripted_error(message),
        }
    }
}

pub(crate) fn scripted_error(message: &str) -> LiveClientError {
    LiveClientError::Redis(redis::RedisError::from(io::Error::other(
        message.to_owned(),
    )))
}

impl AsyncRedisCommands for ScriptedPing {
    type Error = LiveClientError;
    type Reply = ();
    type Pipeline = ();

    fn ping(&self) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        self.calls.fetch_add(1, Ordering::Relaxed);
        match &self.answer {
            ScriptedAnswer::Answer(answer) => ready(Ok(*answer)),
            ScriptedAnswer::Failure(message) => ready(Err(scripted_error(message))),
        }
    }

    fn delete(&self, _names: &[&str]) -> impl Future<Output = Result<i64, Self::Error>> + Send {
        ready(Err(self.reject()))
    }

    fn ttl(&self, _name: &str) -> impl Future<Output = Result<i64, Self::Error>> + Send {
        ready(Err(self.reject()))
    }

    fn expire(
        &self,
        _name: &str,
        _time: i64,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        ready(Err(self.reject()))
    }

    fn rpush(
        &self,
        _name: &str,
        _values: &[RedisScalar],
    ) -> impl Future<Output = Result<i64, Self::Error>> + Send {
        ready(Err(self.reject()))
    }

    fn lpop(
        &self,
        _name: &str,
        _count: Option<i64>,
    ) -> impl Future<Output = Result<Self::Reply, Self::Error>> + Send {
        ready(Err(self.reject()))
    }

    fn pipeline(&self, _transaction: bool) -> Self::Pipeline {}

    fn eval(
        &self,
        _script: &str,
        _numkeys: i64,
        _keys_and_args: &[RedisScalar],
    ) -> impl Future<Output = Result<Self::Reply, Self::Error>> + Send {
        ready(Err(self.reject()))
    }
}

// a port nothing listens on, so a connection attempt is refused at once
pub(crate) fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("an ephemeral port");
    let port = listener.local_addr().expect("the listener address").port();
    drop(listener);
    port
}

// a fresh connection sends CLIENT SETINFO twice before the first command the test scripts
pub(crate) fn script_after_connection_setup(server: &FakeServer, name: &str, reply: Reply) {
    server.script(name, reply.clone());
    server.script(name, reply.clone());
    server.script(name, reply);
}

pub(crate) async fn wait_for_commands(server: &FakeServer, name: &str, count: usize) {
    for _attempt in 0_usize..200 {
        if server.commands_named(name).len() >= count {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("the fake server did not receive {count} {name} commands");
}

#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(crate) fn capture() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let guard = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .set_default();
        (logs, guard)
    }

    pub(crate) fn lines(&self) -> Vec<String> {
        let bytes = self.0.lock().expect("the log buffer").clone();
        String::from_utf8(bytes)
            .expect("UTF-8 log output")
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

impl io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut logged = self
            .0
            .lock()
            .map_err(|_poisoned| io::Error::other("log buffer poisoned"))?;
        logged.extend_from_slice(buf);
        drop(logged);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}
