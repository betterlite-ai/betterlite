use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use serde_json::json;
use tokio::io::{
    AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader, ReadHalf, split,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::helpers::{HelperError, OrHelperError as _};

const DEFAULT_TTL: i64 = 60;

pub(crate) const FAILURE: &[u8] = b"-ERR boom\r\n";

#[derive(Default)]
struct State {
    replies: BTreeMap<String, VecDeque<Vec<u8>>>,
    received: Vec<Vec<String>>,
}

pub(crate) struct FakeRedis {
    pub(crate) addr: SocketAddr,
    state: Arc<Mutex<State>>,
    _tasks: JoinSet<()>,
}

impl FakeRedis {
    pub(crate) async fn start() -> Result<Self, Box<HelperError>> {
        let listener = TcpListener::bind("127.0.0.1:0").await.or_value_error()?;
        let addr = listener.local_addr().or_value_error()?;
        let state = Arc::new(Mutex::new(State::default()));
        let mut tasks = JoinSet::new();
        let accepting = Arc::clone(&state);
        tasks.spawn(async move { accept_loop(listener, accepting).await });
        Ok(Self {
            addr,
            state,
            _tasks: tasks,
        })
    }

    pub(crate) fn script(&self, command: &str, reply: Vec<u8>) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replies
            .entry(command.to_ascii_uppercase())
            .or_default()
            .push_back(reply);
    }

    fn received(&self, name: &str) -> Vec<Vec<String>> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .received
            .iter()
            .filter(|command| command.first().is_some_and(|first| first == name))
            .cloned()
            .collect()
    }
}

pub(crate) fn set_expiries(server: &FakeRedis) -> Vec<serde_json::Value> {
    server
        .received("SET")
        .iter()
        .map(|command| {
            let key = command.get(1).cloned().unwrap_or_default();
            let expiry = command
                .iter()
                .position(|argument| argument == "EX")
                .and_then(|index| command.get(index.saturating_add(1)))
                .cloned()
                .unwrap_or_default();
            serde_json::json!({"key": key, "ex": expiry})
        })
        .collect()
}

pub(crate) fn delete_lengths(server: &FakeRedis) -> Vec<usize> {
    server
        .received("DEL")
        .iter()
        .map(|command| command.len().saturating_sub(1))
        .collect()
}

async fn accept_loop(listener: TcpListener, state: Arc<Mutex<State>>) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            return;
        };
        let owned = Arc::clone(&state);
        connections.spawn(async move { serve(stream, owned).await });
    }
}

async fn serve(stream: TcpStream, state: Arc<Mutex<State>>) {
    let (read_half, mut write_half) = split(stream);
    let mut reader = BufReader::new(read_half);
    while let Some(args) = read_args(&mut reader).await {
        let Some(name) = args.first().map(|argument| argument.to_ascii_uppercase()) else {
            return;
        };
        let argument_count = args.len();
        let reply = {
            let mut held = state.lock().unwrap_or_else(PoisonError::into_inner);
            held.received.push(args);
            held.replies
                .get_mut(&name)
                .and_then(VecDeque::pop_front)
                .unwrap_or_else(|| fallback(&name, argument_count))
        };
        if write_half.write_all(&reply).await.is_err() {
            return;
        }
    }
}

fn fallback(name: &str, argument_count: usize) -> Vec<u8> {
    match name {
        "PING" => b"+PONG\r\n".to_vec(),
        "INFO" => b"$21\r\nredis_version:7.2.0\r\n\r\n".to_vec(),
        "MGET" => {
            let misses = argument_count.saturating_sub(1);
            format!("*{misses}\r\n{}", "$-1\r\n".repeat(misses)).into_bytes()
        }
        "GET" => b"$-1\r\n".to_vec(),
        "INCRBYFLOAT" => b"$3\r\n1.0\r\n".to_vec(),
        "TTL" => format!(":{DEFAULT_TTL}\r\n").into_bytes(),
        "INCRBY" | "DEL" | "EXPIRE" | "SADD" => b":1\r\n".to_vec(),
        _ => b"+OK\r\n".to_vec(),
    }
}

async fn read_args(reader: &mut BufReader<ReadHalf<TcpStream>>) -> Option<Vec<String>> {
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

type Cause<'error> = dyn std::error::Error + Send + Sync + 'error;

struct Allow;

impl ServiceHooks for Allow {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &Cause<'_>) {}
}

impl BreakerGate for Allow {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(1))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &Cause<'_>) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &Cause<'_>) {}
}

pub(crate) async fn connect_cache(addr: SocketAddr) -> Result<Arc<RedisCache>, Box<HelperError>> {
    // every field left out keeps LiteLLM's Redis default; only the fake endpoint and its timeouts are set
    let overrides: RedisOverrides = serde_json::from_value(json!({
        "host": addr.ip().to_string(),
        "port": addr.port(),
        "socket_timeout": 2.0_f64,
        "socket_connect_timeout": 2.0_f64,
    }))
    .or_value_error()?;
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: DEFAULT_TTL,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(Allow),
        gate: Arc::new(Allow),
    };
    let env = BTreeMap::<String, String>::new();
    connect(options, &env).await.map(Arc::new).or_value_error()
}
