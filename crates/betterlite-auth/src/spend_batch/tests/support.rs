use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_db::budget_permission_rows::JsonData;
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
use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;
use tokio::io::{
    AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader, ReadHalf, split,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tracing::Subscriber;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

const SOCKET_TIMEOUT_SECONDS: f64 = 2.0;
const DEFAULT_TTL_SECONDS: i64 = 60;
const DEFAULT_MAJOR_VERSION: u32 = 7;

#[derive(Clone, Debug)]
pub(crate) enum Reply {
    Status(&'static str),
    Bulk(String),
    Nil,
    Array(Vec<Self>),
    Error(String),
}

impl Reply {
    fn encode(&self) -> Vec<u8> {
        match self {
            Self::Status(text) => format!("+{text}\r\n").into_bytes(),
            Self::Bulk(text) => format!("${}\r\n{text}\r\n", text.len()).into_bytes(),
            Self::Nil => b"$-1\r\n".to_vec(),
            Self::Array(items) => {
                let mut encoded = format!("*{}\r\n", items.len()).into_bytes();
                for item in items {
                    encoded.extend(item.encode());
                }
                encoded
            }
            Self::Error(text) => format!("-{text}\r\n").into_bytes(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Command {
    pub(crate) name: String,
    pub(crate) args: Vec<String>,
}

#[derive(Default)]
struct MgetScript {
    replies: VecDeque<Reply>,
    hold: Option<MgetHold>,
}

struct MgetHold {
    arrival: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

pub(crate) struct Held {
    arrival: Option<oneshot::Receiver<()>>,
    release: Option<oneshot::Sender<()>>,
}

impl Held {
    pub(crate) async fn wait_arrived(&mut self) {
        if let Some(arrival) = self.arrival.take() {
            let _received = arrival.await;
        }
    }

    pub(crate) fn release(&mut self) {
        if let Some(release) = self.release.take() {
            let _sent = release.send(());
        }
    }
}

pub(crate) struct FakeRedis {
    addr: SocketAddr,
    commands: Arc<Mutex<Vec<Command>>>,
    mget: Arc<Mutex<MgetScript>>,
    tasks: JoinSet<()>,
}

impl FakeRedis {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the fake Redis listener binds");
        let addr = listener.local_addr().expect("the fake Redis address");
        let commands = Arc::new(Mutex::new(Vec::new()));
        let mget = Arc::new(Mutex::new(MgetScript::default()));
        let mut tasks = JoinSet::new();
        let recorded = Arc::clone(&commands);
        let scripted = Arc::clone(&mget);
        tasks.spawn(async move { accept(listener, &recorded, &scripted).await });
        Self {
            addr,
            commands,
            mget,
            tasks,
        }
    }

    pub(crate) fn script_mget(&self, reply: Reply) {
        lock(&self.mget).replies.push_back(reply);
    }

    pub(crate) fn hold_next_mget(&self) -> Held {
        let (arrival, arrived) = oneshot::channel();
        let (release, released) = oneshot::channel();
        lock(&self.mget).hold = Some(MgetHold {
            arrival,
            release: released,
        });
        Held {
            arrival: Some(arrived),
            release: Some(release),
        }
    }

    pub(crate) fn commands_named(&self, name: &str) -> Vec<Command> {
        lock(&self.commands)
            .iter()
            .filter(|command| command.name == name)
            .cloned()
            .collect()
    }

    pub(crate) fn mget_keys(&self) -> Vec<Vec<String>> {
        self.commands_named("MGET")
            .into_iter()
            .map(|command| command.args[1..].to_vec())
            .collect()
    }
}

impl Drop for FakeRedis {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

async fn accept(
    listener: TcpListener,
    commands: &Arc<Mutex<Vec<Command>>>,
    mget: &Arc<Mutex<MgetScript>>,
) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            return;
        };
        let recorded = Arc::clone(commands);
        let scripted = Arc::clone(mget);
        connections.spawn(async move { serve(stream, &recorded, &scripted).await });
    }
}

async fn serve(stream: TcpStream, commands: &Mutex<Vec<Command>>, mget: &Mutex<MgetScript>) {
    let (read_half, mut write_half) = split(stream);
    let mut reader = BufReader::new(read_half);
    while let Some(args) = read_command(&mut reader).await {
        let Some(name) = args.first().map(|argument| argument.to_ascii_uppercase()) else {
            return;
        };
        let argument_count = args.len();
        lock(commands).push(Command {
            name: name.clone(),
            args,
        });
        let reply = if name == "MGET" {
            take_mget_reply(mget, argument_count).await
        } else {
            default_reply(&name)
        };
        if write_half.write_all(&reply.encode()).await.is_err() {
            return;
        }
    }
}

async fn take_mget_reply(mget: &Mutex<MgetScript>, argument_count: usize) -> Reply {
    let held = lock(mget).hold.take();
    if let Some(hold) = held {
        let _arrived = hold.arrival.send(());
        let _released = hold.release.await;
    }
    let scripted = lock(mget).replies.pop_front();
    scripted.unwrap_or_else(|| Reply::Array(vec![Reply::Nil; argument_count.saturating_sub(1)]))
}

fn default_reply(name: &str) -> Reply {
    match name {
        "PING" => Reply::Status("PONG"),
        "INFO" => Reply::Bulk("redis_version:7.2.0\r\n".to_owned()),
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

struct Refuse;

impl BreakerGate for Refuse {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Err(BreakerOpen { name })
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

pub(crate) async fn allow_cache(server: &FakeRedis) -> Arc<RedisCache> {
    connect_cache(server, Arc::new(Allow)).await
}

pub(crate) async fn refusing_cache(server: &FakeRedis) -> Arc<RedisCache> {
    connect_cache(server, Arc::new(Refuse)).await
}

async fn connect_cache(server: &FakeRedis, gate: Arc<dyn BreakerGate>) -> Arc<RedisCache> {
    let wire = format!(
        r#"{{"host":"{}","port":{},"socket_timeout":{SOCKET_TIMEOUT_SECONDS},"socket_connect_timeout":{SOCKET_TIMEOUT_SECONDS}}}"#,
        server.addr.ip(),
        server.addr.port(),
    );
    let overrides: RedisOverrides = serde_json::from_str(&wire).expect("the Redis overrides parse");
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: DEFAULT_TTL_SECONDS,
        redis_flush_size: None,
        default_major_version: DEFAULT_MAJOR_VERSION,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(Allow),
        gate,
    };
    let env = BTreeMap::<String, String>::new();
    Arc::new(
        connect(options, &env)
            .await
            .expect("the fake Redis accepts the connection"),
    )
}

struct EventCounter(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for EventCounter {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if event.metadata().level() == &tracing::Level::DEBUG {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(crate) struct DebugEvents {
    count: Arc<AtomicUsize>,
    _guard: DefaultGuard,
}

impl DebugEvents {
    pub(crate) fn capture() -> Self {
        let count = Arc::new(AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry().with(EventCounter(Arc::clone(&count)));
        Self {
            count,
            _guard: set_default(subscriber),
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }
}

pub(crate) fn keys(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn decimal(text: &str) -> Decimal {
    text.parse().expect("a fixture decimal parses")
}

pub(crate) fn user_auth(fields: &[(&str, &str)]) -> UserAPIKeyAuth {
    let mut values = BTreeMap::new();
    for (name, value) in fields {
        values.insert((*name).to_owned(), JsonData::String((*value).to_owned()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(values)).expect("the auth fields decode")
}
