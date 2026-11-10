use std::collections::{BTreeMap, VecDeque};
use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::nodes::ClusterNodeInput;
use betterlite_redis::redis_connection_settings::model::scalars::{FloatOrText, IntOrText, Scalar};
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;

use super::input::Reply;

const SOCKET_TIMEOUT_SECONDS: f64 = 1.0;

pub(crate) struct FakeServer {
    pub(crate) addr: SocketAddr,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct Shared {
    replies: Mutex<BTreeMap<String, VecDeque<Reply>>>,
    commands: Mutex<Vec<Vec<String>>>,
    slots: Mutex<Vec<u8>>,
}

impl FakeServer {
    pub(crate) fn start() -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared {
            replies: Mutex::new(BTreeMap::new()),
            commands: Mutex::new(Vec::new()),
            slots: Mutex::new(cluster_slots(addr)),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let owned = Arc::clone(&shared);
        let stopping = Arc::clone(&stop);
        let accept =
            thread::Builder::new().spawn(move || accept_loop(&listener, &owned, &stopping))?;
        Ok(Self {
            addr,
            shared,
            stop,
            accept: Some(accept),
        })
    }

    pub(crate) fn script(&self, name: &str, reply: Reply) {
        match self.shared.replies.lock() {
            Ok(mut replies) => replies
                .entry(name.to_ascii_uppercase())
                .or_default()
                .push_back(reply),
            Err(poisoned) => poisoned
                .into_inner()
                .entry(name.to_ascii_uppercase())
                .or_default()
                .push_back(reply),
        }
    }

    pub(crate) fn commands(&self) -> Vec<Vec<String>> {
        match self.shared.commands.lock() {
            Ok(commands) => commands.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _wake = TcpStream::connect(self.addr);
        if let Some(accept) = self.accept.take() {
            let _joined = accept.join();
        }
    }
}

fn cluster_slots(addr: SocketAddr) -> Vec<u8> {
    let host = addr.ip().to_string();
    let port = addr.port();
    let mut out = b"*1\r\n*3\r\n:0\r\n:16383\r\n".to_vec();
    out.extend(format!("*2\r\n${}\r\n{host}\r\n:{port}\r\n", host.len()).into_bytes());
    out
}

fn accept_loop(listener: &TcpListener, shared: &Arc<Shared>, stop: &Arc<AtomicBool>) {
    for stream in listener.incoming().flatten() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let owned = Arc::clone(shared);
        let _spawned = thread::Builder::new().spawn(move || serve(stream, &owned));
    }
}

fn serve(stream: TcpStream, shared: &Shared) {
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(cloned);
    let mut writer = stream;
    while let Some(command) = read_command(&mut reader) {
        let Some(name) = command.first().cloned() else {
            return;
        };
        match shared.commands.lock() {
            Ok(mut commands) => commands.push(command),
            Err(poisoned) => poisoned.into_inner().push(command),
        }
        let reply = if name == "CLUSTER" {
            match shared.slots.lock() {
                Ok(slots) => slots.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            }
        } else {
            let scripted = match shared.replies.lock() {
                Ok(mut replies) => replies.get_mut(&name).and_then(VecDeque::pop_front),
                Err(poisoned) => poisoned
                    .into_inner()
                    .get_mut(&name)
                    .and_then(VecDeque::pop_front),
            };
            scripted
                .unwrap_or_else(|| Reply::Simple("OK".to_owned()))
                .encode()
        };
        if writer.write_all(&reply).is_err() {
            return;
        }
    }
}

fn read_command(reader: &mut BufReader<TcpStream>) -> Option<Vec<String>> {
    let mut header = String::new();
    if reader.read_line(&mut header).ok()? == 0 {
        return None;
    }
    let count: usize = header.trim().strip_prefix('*')?.parse().ok()?;
    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let length: usize = line.trim().strip_prefix('$')?.parse().ok()?;
        let mut bytes = vec![0_u8; length.saturating_add(2)];
        reader.read_exact(&mut bytes).ok()?;
        bytes.truncate(length);
        args.push(String::from_utf8(bytes).ok()?);
    }
    Some(args)
}

impl Reply {
    pub(crate) fn encode(&self) -> Vec<u8> {
        match self {
            Self::Bulk(text) => format!("${}\r\n{text}\r\n", text.len()).into_bytes(),
            Self::Simple(text) => format!("+{text}\r\n").into_bytes(),
            Self::Int(value) => format!(":{value}\r\n").into_bytes(),
            Self::Error(text) => format!("-{text}\r\n").into_bytes(),
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

struct Silent;

impl ServiceHooks for Silent {
    fn success(&self, _event: &ServiceEvent) {}
    fn failure(&self, _event: &ServiceEvent, _error: &(dyn std::error::Error + Send + Sync)) {}
}

struct Allow;

impl BreakerGate for Allow {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(1))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn std::error::Error + Send + Sync)) {}

    fn swallowed(
        &self,
        _admission: &BreakerAdmission,
        _error: &(dyn std::error::Error + Send + Sync),
    ) {
    }
}

const fn blank_overrides() -> RedisOverrides {
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

pub(crate) async fn connect_cache(
    server: Option<&FakeServer>,
    namespace: Option<&str>,
    default_ttl: i64,
    cluster: bool,
) -> Result<Arc<RedisCache>, String> {
    let (host, port) = server.map_or_else(
        || ("127.0.0.1".to_owned(), 1),
        |node| (node.addr.ip().to_string(), node.addr.port()),
    );
    let mut overrides = blank_overrides();
    overrides.socket_timeout = Some(FloatOrText::Value(SOCKET_TIMEOUT_SECONDS));
    overrides.socket_connect_timeout = Some(FloatOrText::Value(SOCKET_TIMEOUT_SECONDS));
    let family = if cluster {
        overrides.startup_nodes = Some(Scalar::Value(vec![ClusterNodeInput {
            host,
            port: IntOrText::Value(i64::from(port)),
        }]));
        RedisCacheFamily::Cluster
    } else {
        overrides.host = Some(host);
        overrides.port = Some(IntOrText::Value(i64::from(port)));
        RedisCacheFamily::Standalone
    };
    let options = RedisCacheOptions {
        overrides,
        family,
        namespace: namespace.map(str::to_owned),
        default_ttl,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(Silent),
        gate: Arc::new(Allow),
    };
    let env = BTreeMap::<String, String>::new();
    connect(options, &env)
        .await
        .map(Arc::new)
        .map_err(|error| error.to_string())
}
