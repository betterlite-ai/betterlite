use std::collections::BTreeMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

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
use tokio::io::{
    AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader, ReadHalf, split,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

const SOCKET_TIMEOUT_SECONDS: f64 = 2.0;
const DEFAULT_TTL_SECONDS: i64 = 60;

#[derive(Debug, Clone)]
pub(crate) enum Reply {
    Status(&'static str),
    Integer(i64),
    Bulk(String),
    Nil,
    Array(Vec<Self>),
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
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Command {
    pub(crate) name: String,
    pub(crate) args: Vec<String>,
}

pub(crate) struct FakeRedis {
    pub(crate) addr: SocketAddr,
    commands: Arc<Mutex<Vec<Command>>>,
    tasks: JoinSet<()>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeRedis {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the fake Redis listener binds");
        let addr = listener.local_addr().expect("the fake Redis address");
        let commands = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = JoinSet::new();
        let recorded = Arc::clone(&commands);
        tasks.spawn(async move { accept(listener, recorded).await });
        Self {
            addr,
            commands,
            tasks,
        }
    }

    pub(crate) fn commands(&self, name: &str) -> Vec<Command> {
        lock(&self.commands)
            .iter()
            .filter(|command| command.name == name)
            .cloned()
            .collect()
    }
}

impl Drop for FakeRedis {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

async fn accept(listener: TcpListener, commands: Arc<Mutex<Vec<Command>>>) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            return;
        };
        let recorded = Arc::clone(&commands);
        connections.spawn(async move { serve(stream, recorded).await });
    }
}

async fn serve(stream: TcpStream, commands: Arc<Mutex<Vec<Command>>>) {
    let (read_half, mut write_half) = split(stream);
    let mut reader = BufReader::new(read_half);
    while let Some(args) = read_command(&mut reader).await {
        let Some(name) = args.first().map(|argument| argument.to_ascii_uppercase()) else {
            return;
        };
        let argument_count = args.len();
        lock(&commands).push(Command {
            name: name.clone(),
            args,
        });
        let reply = default_reply(&name, argument_count);
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
        "TTL" => Reply::Integer(DEFAULT_TTL_SECONDS),
        "SET" | "FLUSHALL" => Reply::Status("OK"),
        "INCRBY" | "INCRBYFLOAT" | "DEL" | "EXPIRE" | "SADD" => Reply::Integer(1),
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
    for _index in 0..count {
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

pub(crate) async fn connect_cache(addr: SocketAddr) -> Arc<RedisCache> {
    let wire = format!(
        r#"{{"host":"{}","port":{},"socket_timeout":{SOCKET_TIMEOUT_SECONDS},"socket_connect_timeout":{SOCKET_TIMEOUT_SECONDS}}}"#,
        addr.ip(),
        addr.port(),
    );
    let overrides: RedisOverrides = serde_json::from_str(&wire).expect("the Redis overrides parse");
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
