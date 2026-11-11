use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use num_traits::ToPrimitive as _;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::scalars::{FloatOrText, IntOrText};
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use tokio::io::{
    AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader, ReadHalf, split,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::backend::memory::MemoryCacheOps;
use crate::backend::memory::local::LocalMemoryOps;
use crate::backend::{BackendError, CacheResult, Ttl};
use crate::dual_cache::{
    CacheOptions, DEFAULT_MAX_SIZE_IN_MEMORY, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE,
    MemoryMaxSize,
};

pub(crate) fn write(ttl: Option<f64>, local_only: bool) -> CacheOptions {
    CacheOptions {
        ttl,
        local_only,
        refresh_ttl: false,
    }
}

pub(crate) const DEFAULT_TTL_SECONDS: i64 = 60;
const SOCKET_TIMEOUT_SECONDS: f64 = 2.0;

pub(crate) fn settings_for_tests(default_in_memory_ttl: Option<f64>) -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    }
}

pub(crate) fn clock_at(seconds: f64) -> Arc<dyn Fn() -> f64 + Send + Sync> {
    Arc::new(move || seconds)
}

pub(crate) fn local_memory(clock: f64) -> Arc<LocalMemoryOps> {
    let memory = LocalMemoryOps::new(1024, clock_at(clock)).expect("the local memory tier builds");
    Arc::new(memory)
}

#[derive(Clone, Debug)]
pub(crate) enum Reply {
    Status(&'static str),
    Integer(i64),
    Bulk(String),
    Nil,
    Array(Vec<Self>),
    Error(&'static str),
}

impl Reply {
    fn encode(&self) -> Vec<u8> {
        match self {
            Self::Status(text) => format!("+{text}\r\n").into_bytes(),
            Self::Integer(number) => format!(":{number}\r\n").into_bytes(),
            Self::Bulk(text) => format!("${}\r\n{text}\r\n", text.len()).into_bytes(),
            Self::Nil => b"$-1\r\n".to_vec(),
            Self::Array(items) => {
                let mut encoded = format!("*{}\r\n", items.len()).into_bytes();
                for item in items {
                    encoded.extend(item.encode());
                }
                encoded
            }
            Self::Error(message) => format!("-{message}\r\n").into_bytes(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Command {
    pub name: String,
    pub args: Vec<String>,
}

pub(crate) struct Shared {
    script: Mutex<BTreeMap<String, VecDeque<Reply>>>,
    commands: Mutex<Vec<Command>>,
}

impl Shared {
    pub(crate) fn take(&self, name: &str) -> Option<Reply> {
        lock(&self.script)
            .get_mut(name)
            .and_then(VecDeque::pop_front)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct FakeRedis {
    pub(crate) addr: SocketAddr,
    pub(crate) shared: Arc<Shared>,
    pub(crate) tasks: JoinSet<()>,
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
            .expect("the fake Redis listener binds");
        let addr = listener.local_addr().expect("the fake Redis address");
        let shared = Arc::new(Shared {
            script: Mutex::new(BTreeMap::new()),
            commands: Mutex::new(Vec::new()),
        });
        let mut tasks = JoinSet::new();
        let accepting = Arc::clone(&shared);
        tasks.spawn(async move { accept(listener, accepting).await });
        Self {
            addr,
            shared,
            tasks,
        }
    }

    pub(crate) fn script(&self, command: &str, replies: Vec<Reply>) {
        lock(&self.shared.script)
            .entry(command.to_ascii_uppercase())
            .or_default()
            .extend(replies);
    }

    pub(crate) fn commands(&self, name: &str) -> Vec<Command> {
        lock(&self.shared.commands)
            .iter()
            .filter(|command| command.name == name)
            .cloned()
            .collect()
    }
}

async fn accept(listener: TcpListener, shared: Arc<Shared>) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            return;
        };
        let owned = Arc::clone(&shared);
        connections.spawn(async move { serve(stream, owned).await });
    }
}

async fn serve(stream: TcpStream, shared: Arc<Shared>) {
    let (read_half, mut write_half) = split(stream);
    let mut reader = BufReader::new(read_half);
    while let Some(args) = read_command(&mut reader).await {
        let Some(name) = args.first().map(|argument| argument.to_ascii_uppercase()) else {
            return;
        };
        let argument_count = args.len();
        lock(&shared.commands).push(Command {
            name: name.clone(),
            args,
        });
        let reply = shared
            .take(&name)
            .unwrap_or_else(|| default_reply(&name, argument_count));
        if write_half.write_all(&reply.encode()).await.is_err() {
            return;
        }
    }
}

fn default_reply(name: &str, argument_count: usize) -> Reply {
    match name {
        "PING" => Reply::Status("PONG"),
        "INFO" => Reply::Bulk("redis_version:7.2.0\r\n".to_owned()),
        "MGET" => Reply::Array(vec![Reply::Nil; argument_count.saturating_sub(1)]),
        "GET" => Reply::Nil,
        "INCRBYFLOAT" => Reply::Bulk("1.0".to_owned()),
        "TTL" => Reply::Integer(DEFAULT_TTL_SECONDS),
        "SET" | "FLUSHALL" => Reply::Status("OK"),
        "INCRBY" | "DEL" | "EXPIRE" | "SADD" => Reply::Integer(1),
        // redis-rs may send CLIENT SETINFO on connect; its reply is ignored
        _ => Reply::Bulk("OK".to_owned()),
    }
}

async fn read_command(reader: &mut BufReader<ReadHalf<TcpStream>>) -> Option<Vec<String>> {
    let mut header = String::new();
    if reader.read_line(&mut header).await.ok()? == 0 {
        return None;
    }
    let count: usize = header.trim().strip_prefix('*')?.parse().ok()?;
    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        let mut line = String::new();
        if reader.read_line(&mut line).await.ok()? == 0 {
            return None;
        }
        let length: usize = line.trim().strip_prefix('$')?.parse().ok()?;
        let mut bytes = vec![0_u8; length.checked_add(2)?];
        reader.read_exact(&mut bytes).await.ok()?;
        bytes.truncate(length);
        args.push(String::from_utf8(bytes).ok()?);
    }
    Some(args)
}

struct Allow;

impl ServiceHooks for Allow {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

impl BreakerGate for Allow {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(1))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

// LiteLLM's RedisCache defaults, spelled out because a struct update hides which fields stay unset
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

pub(crate) async fn connect_cache(addr: SocketAddr) -> Arc<RedisCache> {
    let mut overrides = blank_overrides();
    overrides.host = Some(addr.ip().to_string());
    overrides.port = Some(IntOrText::Value(i64::from(addr.port())));
    overrides.socket_timeout = Some(FloatOrText::Value(SOCKET_TIMEOUT_SECONDS));
    overrides.socket_connect_timeout = Some(FloatOrText::Value(SOCKET_TIMEOUT_SECONDS));
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: DEFAULT_TTL_SECONDS,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(Allow),
        gate: Arc::new(Allow),
    };
    let env = BTreeMap::<String, String>::new();
    let cache = connect(options, &env)
        .await
        .expect("the fake Redis accepts the connection");
    Arc::new(cache)
}

// a memory tier that records the live max size so the resize path can be observed
pub(crate) struct ResizableMemory {
    values: Mutex<BTreeMap<String, (CacheValue, Option<f64>)>>,
    max_size: Mutex<usize>,
}

impl ResizableMemory {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            values: Mutex::new(BTreeMap::new()),
            max_size: Mutex::new(DEFAULT_MAX_SIZE_IN_MEMORY),
        })
    }

    pub(crate) fn max_size(&self) -> usize {
        *lock(&self.max_size)
    }
}

impl MemoryCacheOps for ResizableMemory {
    fn default_ttl(&self) -> i64 {
        600
    }

    fn get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        Ok(lock(&self.values)
            .get(key)
            .map(|(value, _ttl)| value.clone()))
    }

    fn set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        lock(&self.values).insert(key.to_owned(), (value, ttl));
        Ok(())
    }

    fn delete_cache(&self, key: &str) -> CacheResult<()> {
        lock(&self.values).remove(key);
        Ok(())
    }

    fn flush_cache(&self) -> CacheResult<()> {
        lock(&self.values).clear();
        Ok(())
    }

    fn batch_get_cache(&self, keys: &[String]) -> CacheResult<Vec<Option<CacheValue>>> {
        let values = lock(&self.values);
        Ok(keys
            .iter()
            .map(|key| values.get(key).map(|(value, _ttl)| value.clone()))
            .collect())
    }

    fn increment_cache(&self, key: &str, value: f64, ttl: Ttl) -> CacheResult<f64> {
        let next = {
            let mut values = lock(&self.values);
            let base = match values.get(key).map(|(stored, _ttl)| stored.clone()) {
                Some(CacheValue::Integer(number)) => {
                    number.to_f64().ok_or_else(|| BackendError::NotNumeric {
                        key: key.to_owned(),
                    })?
                }
                Some(CacheValue::Float(number)) => number,
                None => 0.0_f64,
                Some(_) => {
                    return Err(BackendError::NotNumeric {
                        key: key.to_owned(),
                    });
                }
            };
            let incremented = base.mul_add(1.0_f64, value);
            values.insert(key.to_owned(), (CacheValue::Float(incremented), ttl));
            incremented
        };
        Ok(next)
    }

    fn ttl_entries(&self) -> CacheResult<Vec<(String, f64)>> {
        let values = lock(&self.values);
        Ok(values
            .iter()
            .filter_map(|(key, (_value, ttl))| ttl.map(|expiry| (key.clone(), expiry)))
            .collect())
    }
}

impl MemoryMaxSize for ResizableMemory {
    fn set_max_size(&self, max_size: usize) {
        *lock(&self.max_size) = max_size;
    }
}
