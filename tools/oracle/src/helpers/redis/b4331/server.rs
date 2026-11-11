use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

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

use super::input::Reply;
use super::scenario::Failure;

const DEFAULT_TTL: i64 = 60;
const SOCKET_TIMEOUT_SECONDS: f64 = 1.0;

pub(crate) struct FakeServer {
    pub(crate) addr: SocketAddr,
    shared: Arc<Shared>,
    tasks: JoinSet<()>,
}

#[derive(Default)]
struct Shared {
    replies: Mutex<BTreeMap<String, VecDeque<Reply>>>,
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

impl FakeServer {
    pub(crate) async fn start() -> Result<Self, Failure> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared::default());
        let mut tasks = JoinSet::new();
        let owned = Arc::clone(&shared);
        tasks.spawn(async move { accept(listener, owned).await });
        Ok(Self {
            addr,
            shared,
            tasks,
        })
    }

    pub(crate) fn script(&self, command: &str, reply: Reply) {
        let name = command.to_ascii_uppercase();
        lock(&self.shared.replies)
            .entry(name)
            .or_default()
            .push_back(reply);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
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
    while let Some(command) = read_command(&mut reader).await {
        let Some(name) = command.first().cloned() else {
            return;
        };
        let reply = lock(&shared.replies)
            .get_mut(&name)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| Reply::Bulk("OK".to_owned()));
        if write_half.write_all(&reply.encode()).await.is_err() {
            return;
        }
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
        let mut bytes = vec![0_u8; length.saturating_add(2)];
        reader.read_exact(&mut bytes).await.ok()?;
        bytes.truncate(length);
        args.push(String::from_utf8(bytes).ok()?);
    }
    Some(args)
}

impl Reply {
    fn encode(&self) -> Vec<u8> {
        match self {
            Self::Bulk(text) => format!("${}\r\n{text}\r\n", text.len()).into_bytes(),
            Self::Nil => b"$-1\r\n".to_vec(),
            Self::Array(items) => {
                let mut out = format!("*{}\r\n", items.len()).into_bytes();
                for item in items {
                    out.extend(item.encode());
                }
                out
            }
        }
    }
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

pub(crate) async fn connect_cache(
    server: &FakeServer,
    namespace: Option<&str>,
) -> Result<Arc<RedisCache>, Failure> {
    let mut overrides = RedisOverrides::default();
    set_endpoint(&mut overrides, server.addr);
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: namespace.map(str::to_owned),
        default_ttl: DEFAULT_TTL,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(Allow),
        gate: Arc::new(Allow),
    };
    let env = BTreeMap::<String, String>::new();
    connect(options, &env)
        .await
        .map(Arc::new)
        .map_err(Failure::Cache)
}

fn set_endpoint(overrides: &mut RedisOverrides, addr: SocketAddr) {
    overrides.host = Some(addr.ip().to_string());
    overrides.port = Some(IntOrText::Value(i64::from(addr.port())));
    overrides.socket_timeout = Some(FloatOrText::Value(SOCKET_TIMEOUT_SECONDS));
    overrides.socket_connect_timeout = Some(FloatOrText::Value(SOCKET_TIMEOUT_SECONDS));
}
