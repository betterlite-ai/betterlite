#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_connection_settings::model::scalars::IntOrText;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use futures_util::future::{Either, select};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::backend::{CacheBackend, CacheResult};
use crate::configuration::cache_module_functions::{CacheSetup, CacheSwitches};
use crate::configuration::callback_registration::CacheCallbacks;
use crate::configuration::settings::{
    CacheDependencies, CacheSettings, DEFAULT_REDIS_MAJOR_VERSION, DEFAULT_REDIS_TTL_SECONDS,
};
use crate::types::cache_type::{CacheMode, LiteLlmCacheType};
use crate::types::call_type::SUPPORTED_CALL_TYPES;
use crate::types::semantic::SemanticCacheScope;

pub(crate) const NOW: f64 = 1_700_000_000.0;

#[derive(Default)]
pub(crate) struct RecordingLogger {
    pub(crate) records: Mutex<Vec<Payload>>,
}

impl LoggerSink for RecordingLogger {
    fn debug(&self, payload: &Payload) -> bool {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(payload.clone());
        true
    }

    fn info(&self, _payload: &Payload) -> bool {
        false
    }

    fn error(&self, payload: &Payload) -> bool {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(payload.clone());
        true
    }

    fn debug_enabled(&self) -> bool {
        true
    }
}

#[derive(Default)]
pub(crate) struct RecordingConsole {
    pub(crate) prints: Mutex<Vec<Payload>>,
}

impl ConsoleSink for RecordingConsole {
    fn print(&self, payload: &Payload) -> bool {
        self.prints
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(payload.clone());
        true
    }
}

pub(crate) type Hooks = VerboseHooks<RecordingLogger, RecordingConsole>;
pub(crate) type Switches = CacheSwitches<RecordingLogger, RecordingConsole>;

pub(crate) fn hooks() -> Hooks {
    VerboseHooks {
        logger: RecordingLogger::default(),
        console: RecordingConsole::default(),
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("fixture redactor compiles"),
        ),
    }
}

pub(crate) fn switches() -> Switches {
    CacheSwitches {
        cache: None,
        callbacks: CacheCallbacks::default(),
        hooks: hooks(),
    }
}

pub(crate) fn messages(hooks: &Hooks) -> Vec<String> {
    let records = hooks
        .logger
        .records
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    records.iter().filter_map(payload_text).collect()
}

fn payload_text(payload: &Payload) -> Option<String> {
    match payload {
        Payload::String(text) => Some(text.clone()),
        Payload::Null
        | Payload::Bool(_)
        | Payload::Number(_)
        | Payload::Array(_)
        | Payload::Object(_) => None,
    }
}

pub(crate) fn settings(cache_type: LiteLlmCacheType) -> CacheSettings {
    CacheSettings {
        cache_type,
        mode: CacheMode::DefaultOn,
        namespace: None,
        ttl: None,
        default_in_memory_ttl: None,
        default_in_redis_ttl: None,
        supported_call_types: SUPPORTED_CALL_TYPES.to_vec(),
        semantic_cache_scope: SemanticCacheScope::Key,
        redis: RedisOverrides::default(),
        redis_flush_size: None,
        redis_default_ttl: DEFAULT_REDIS_TTL_SECONDS,
        redis_default_major_version: DEFAULT_REDIS_MAJOR_VERSION,
        disk_cache_dir: None,
        backend: None,
    }
}

pub(crate) fn redis_settings(server: &FakeRedis) -> CacheSettings {
    let mut settings = settings(LiteLlmCacheType::Redis);
    settings.redis.host = Some(server.host());
    settings.redis.port = Some(IntOrText::Value(i64::from(server.port())));
    settings
}

pub(crate) fn environment() -> BTreeMap<String, String> {
    BTreeMap::new()
}

pub(crate) fn setup<'setup>(
    settings: CacheSettings,
    environment: &'setup BTreeMap<String, String>,
    dependencies: &'setup CacheDependencies,
) -> CacheSetup<'setup> {
    CacheSetup {
        settings,
        environment,
        dependencies,
    }
}

pub(crate) fn dependencies() -> CacheDependencies {
    dependencies_with_gate(Arc::new(ClosedBreakerGate))
}

pub(crate) fn rejecting_dependencies() -> CacheDependencies {
    dependencies_with_gate(Arc::new(RejectingGate))
}

fn dependencies_with_gate(redis_gate: Arc<dyn BreakerGate>) -> CacheDependencies {
    CacheDependencies {
        clock: Arc::new(move || NOW),
        redis_factory: Arc::new(LiveClientFactory::new(None)),
        redis_store: Arc::new(RedisClientStore::new()),
        redis_hooks: Arc::new(SilentServiceHooks),
        redis_gate,
    }
}

pub(crate) struct SilentServiceHooks;

impl ServiceHooks for SilentServiceHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

pub(crate) struct ClosedBreakerGate;

impl BreakerGate for ClosedBreakerGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

pub(crate) struct RejectingGate;

impl BreakerGate for RejectingGate {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Err(BreakerOpen { name })
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

#[derive(Clone, Default)]
pub(crate) struct DisconnectLog(Arc<Mutex<u32>>);

impl DisconnectLog {
    pub(crate) fn count(&self) -> u32 {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self) {
        let mut count = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *count = count.saturating_add(1);
    }
}

#[derive(Default)]
struct TestBackend {
    disconnects: DisconnectLog,
}

#[async_trait]
impl CacheBackend for TestBackend {
    fn default_ttl(&self) -> i64 {
        DEFAULT_REDIS_TTL_SECONDS
    }

    async fn disconnect(&self) -> CacheResult<()> {
        self.disconnects.record();
        Ok(())
    }
}

pub(crate) fn test_backend() -> (Arc<dyn CacheBackend>, DisconnectLog) {
    let backend = Arc::new(TestBackend::default());
    let log = backend.disconnects.clone();
    let injected: Arc<dyn CacheBackend> = backend;
    (injected, log)
}

pub(crate) struct FakeRedis {
    address: SocketAddr,
    commands: Arc<Mutex<Vec<Recorded>>>,
    connections: Arc<AtomicUsize>,
    tasks: JoinSet<()>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Recorded {
    pub(crate) name: String,
    pub(crate) arguments: Vec<String>,
}

impl Drop for FakeRedis {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

impl FakeRedis {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("an ephemeral listener");
        let address = listener.local_addr().expect("the listener address");
        let commands = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let mut tasks = JoinSet::new();
        tasks.spawn(serve_listener(
            listener,
            Arc::clone(&commands),
            Arc::clone(&connections),
        ));
        Self {
            address,
            commands,
            connections,
            tasks,
        }
    }

    pub(crate) fn host(&self) -> String {
        self.address.ip().to_string()
    }

    pub(crate) fn port(&self) -> u16 {
        self.address.port()
    }

    pub(crate) fn connections(&self) -> usize {
        self.connections.load(Ordering::Relaxed)
    }

    pub(crate) fn commands_named(&self, name: &str) -> Vec<Recorded> {
        self.commands
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|recorded| recorded.name == name)
            .cloned()
            .collect()
    }
}

pub(crate) async fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral listener");
    let address = listener.local_addr().expect("the listener address");
    drop(listener);
    address.port()
}

async fn serve_listener(
    listener: TcpListener,
    commands: Arc<Mutex<Vec<Recorded>>>,
    connections: Arc<AtomicUsize>,
) {
    let mut serving = JoinSet::new();
    let mut pending = 0_usize;
    loop {
        let accepted_connection = if pending == 0 {
            Some(listener.accept().await)
        } else {
            let accepting = Box::pin(listener.accept());
            let joined = Box::pin(serving.join_next());
            match select(accepting, joined).await {
                Either::Left((connection, _pending_join)) => Some(connection),
                Either::Right((_completed, _pending_accept)) => {
                    pending = pending.saturating_sub(1);
                    None
                }
            }
        };
        let Some(next_connection) = accepted_connection else {
            continue;
        };
        let Ok((stream, _peer)) = next_connection else {
            return;
        };
        accept_connection(stream, &commands, &connections, &mut serving);
        pending = pending.saturating_add(1);
    }
}

fn accept_connection(
    stream: TcpStream,
    commands: &Arc<Mutex<Vec<Recorded>>>,
    connections: &Arc<AtomicUsize>,
    serving: &mut JoinSet<()>,
) {
    connections.fetch_add(1, Ordering::Relaxed);
    serving.spawn(serve_connection(stream, Arc::clone(commands)));
}

async fn serve_connection(stream: TcpStream, commands: Arc<Mutex<Vec<Recorded>>>) {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    while let Ok(Some(command)) = read_command(&mut reader).await {
        let reply = reply_for(&command);
        commands
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                name: command.name,
                arguments: command.arguments,
            });
        if write_half.write_all(&reply).await.is_err() {
            return;
        }
        if write_half.flush().await.is_err() {
            return;
        }
    }
}

struct Command {
    name: String,
    arguments: Vec<String>,
}

async fn read_command<R>(reader: &mut R) -> io::Result<Option<Command>>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    let Some(count_text) = line.trim().strip_prefix('*') else {
        return Ok(None);
    };
    let count: usize = count_text
        .parse()
        .map_err(|_error| io::Error::other("the fake needs an array command"))?;
    let mut arguments = Vec::with_capacity(count);
    for _ in 0..count {
        let mut header = String::new();
        if reader.read_line(&mut header).await? == 0 {
            return Ok(None);
        }
        let Some(length_text) = header.trim().strip_prefix('$') else {
            return Ok(None);
        };
        let length: usize = length_text
            .parse()
            .map_err(|_error| io::Error::other("the fake needs a bulk string"))?;
        let mut buffer = vec![0_u8; length.saturating_add(2)];
        reader.read_exact(&mut buffer).await?;
        buffer.truncate(length);
        let argument = String::from_utf8(buffer)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        arguments.push(argument);
    }
    let Some(first) = arguments.first() else {
        return Ok(None);
    };
    let name = first.to_ascii_uppercase();
    Ok(Some(Command { name, arguments }))
}

fn reply_for(command: &Command) -> Vec<u8> {
    match command.name.as_str() {
        "PING" => b"+PONG\r\n".to_vec(),
        "DEL" => format!(":{}\r\n", command.arguments.len().saturating_sub(1)).into_bytes(),
        _ => b"+OK\r\n".to_vec(),
    }
}
