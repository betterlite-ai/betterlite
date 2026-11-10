use std::collections::BTreeMap;
use std::error::Error;
use std::io::{self, BufRead, BufReader, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use betterlite_redis::redis_cache_values::{RedisCacheValues, RedisCacheValuesSettings};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::{RedisCacheFamily, RedisTransport};
use betterlite_redis::redis_connection_settings::model::scalars::Scalar;
use betterlite_redis::redis_connection_settings::model::settings::{
    ClientKind, RedisConnectionSettings, RedisOverrides,
};
use betterlite_redis::redis_connection_settings::resolve::resolve_redis_kwargs;
use betterlite_redis::redis_live_client::LiveClientFactory;
use serde_json::value::RawValue;
use tokio::runtime::Builder;

use crate::http_scenarios::error::ScenarioError;
use crate::http_scenarios::run::PollingTransport;

const SOCKET_TIMEOUT: f64 = 10.0;

#[derive(Clone)]
pub(super) struct StoredEntry {
    pub(super) value: String,
    pub(super) ttl_seconds: Option<i64>,
}

type Registry = BTreeMap<String, StoredEntry>;
type Entries = Arc<Mutex<Registry>>;

// B1139 provisions the Redis the two peers seed; no redis-server binary exists in the gate image.
pub(super) struct RedisServer {
    addr: SocketAddr,
    entries: Entries,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl RedisServer {
    pub(super) fn start() -> Result<Self, ScenarioError> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(ScenarioError::Io)?;
        let addr = listener.local_addr().map_err(ScenarioError::Io)?;
        let entries: Entries = Arc::new(Mutex::new(Registry::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::clone(&entries);
        let stopping = Arc::clone(&stop);
        let accept = thread::Builder::new()
            .spawn(move || accept_loop(&listener, &shared, &stopping))
            .map_err(ScenarioError::Io)?;
        Ok(Self {
            addr,
            entries,
            stop,
            accept: Some(accept),
        })
    }

    pub(super) fn url(&self) -> String {
        format!("redis://{}/0", self.addr)
    }

    pub(super) fn polling(&self) -> RedisPolling {
        RedisPolling {
            addr: self.addr,
            entries: Arc::clone(&self.entries),
        }
    }
}

impl Drop for RedisServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _wake = TcpStream::connect(self.addr);
        if let Some(accept) = self.accept.take() {
            let _joined = accept.join();
        }
    }
}

pub(super) struct RedisPolling {
    addr: SocketAddr,
    entries: Entries,
}

impl PollingTransport for RedisPolling {
    fn write(
        &mut self,
        prefix: &str,
        key: &str,
        value: &str,
        ttl_seconds: u64,
    ) -> Result<(), ScenarioError> {
        store_polling(self, &format!("{prefix}:{key}"), value, ttl_seconds)
    }
}

impl RedisPolling {
    pub(super) fn entry(&self, key: &str) -> Option<StoredEntry> {
        locked(&self.entries).get(key).cloned()
    }
}

// async_set_cache swallows a connection failure, so the server's own record is the proof it stored anything.
fn store_polling(
    polling: &RedisPolling,
    full_key: &str,
    value: &str,
    ttl_seconds: u64,
) -> Result<(), ScenarioError> {
    let entry = RawValue::from_string(value.to_owned()).map_err(ScenarioError::Json)?;
    let cache = cache(polling.addr)?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(ScenarioError::Io)?;
    let ttl = Duration::from_secs(ttl_seconds).as_secs_f64();
    runtime
        .block_on(cache.async_set_cache(Some(full_key), &entry, Some(ttl), false))
        .map_err(|error| ScenarioError::Message {
            message: format!("scenario polling entry {full_key} failed: {error}"),
        })?;
    let expected = i64::try_from(ttl_seconds).ok();
    let stored = polling.entry(full_key);
    if stored.is_some_and(|record| record.value == value && record.ttl_seconds == expected) {
        return Ok(());
    }
    Err(ScenarioError::Message {
        message: "scenario polling entry was not stored with the expected value and TTL".to_owned(),
    })
}

fn cache(addr: SocketAddr) -> Result<RedisCacheValues, ScenarioError> {
    let settings = connection_settings(addr)?;
    let transport = RedisTransport::new(
        settings,
        RedisCacheFamily::Standalone,
        Arc::new(LiveClientFactory::new(None)),
        Arc::new(RedisClientStore::new()),
        None,
    )
    .map_err(|error| ScenarioError::Message {
        message: format!("scenario polling transport failed: {error}"),
    })?;
    Ok(RedisCacheValues::new(
        RedisCacheValuesSettings {
            namespace: None,
            default_ttl: None,
            redis_flush_size: None,
        },
        Arc::new(transport),
        Arc::new(AllowHooks),
        Arc::new(AllowGate),
    ))
}

fn connection_settings(addr: SocketAddr) -> Result<RedisConnectionSettings, ScenarioError> {
    let overrides = RedisOverrides {
        host: Some(addr.ip().to_string()),
        port: Some(Scalar::Value(i64::from(addr.port()))),
        socket_timeout: Some(Scalar::Value(SOCKET_TIMEOUT)),
        socket_connect_timeout: Some(Scalar::Value(SOCKET_TIMEOUT)),
        ..RedisOverrides::default()
    };
    let environment: BTreeMap<String, String> = BTreeMap::new();
    resolve_redis_kwargs(overrides, ClientKind::Async, &environment).map_err(|error| {
        ScenarioError::Message {
            message: format!("scenario polling settings failed: {error}"),
        }
    })
}

struct AllowHooks;

impl ServiceHooks for AllowHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

struct AllowGate;

impl BreakerGate for AllowGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

fn accept_loop(listener: &TcpListener, entries: &Entries, stop: &Arc<AtomicBool>) {
    while let Ok((stream, _peer)) = listener.accept() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let shared = Arc::clone(entries);
        let _served = thread::Builder::new().spawn(move || serve(stream, &shared));
    }
}

fn serve(stream: TcpStream, entries: &Entries) {
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(cloned);
    let mut writer = stream;
    while let Ok(Some(command)) = read_command(&mut reader) {
        let reply = match command.name.as_str() {
            "SET" => store(entries, &command),
            "GET" => lookup(entries, &command),
            "PING" => b"+PONG\r\n".to_vec(),
            _ => b"+OK\r\n".to_vec(),
        };
        if writer.write_all(&reply).is_err() {
            return;
        }
    }
}

struct Command {
    name: String,
    args: Vec<String>,
}

fn read_command(reader: &mut impl BufRead) -> io::Result<Option<Command>> {
    let Some(count) = read_header(reader, '*')? else {
        return Ok(None);
    };
    let mut args = Vec::new();
    for _ in 0..count {
        let Some(length) = read_header(reader, '$')? else {
            return Ok(None);
        };
        args.push(read_argument(reader, length)?);
    }
    let Some(name) = args.first() else {
        return Ok(None);
    };
    Ok(Some(Command {
        name: name.to_ascii_uppercase(),
        args,
    }))
}

fn read_header(reader: &mut impl BufRead, marker: char) -> io::Result<Option<usize>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    line.trim().strip_prefix(marker).map_or(Ok(None), |text| {
        text.parse::<usize>()
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    })
}

fn read_argument(reader: &mut impl BufRead, length: usize) -> io::Result<String> {
    let mut bytes = vec![0_u8; length.saturating_add(2)];
    reader.read_exact(&mut bytes)?;
    bytes.truncate(length);
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn store(entries: &Entries, command: &Command) -> Vec<u8> {
    let (Some(key), Some(value)) = (command.args.get(1), command.args.get(2)) else {
        return b"-ERR wrong number of arguments for 'set' command\r\n".to_vec();
    };
    let entry = StoredEntry {
        value: value.clone(),
        ttl_seconds: expiry(&command.args),
    };
    locked(entries).insert(key.clone(), entry);
    b"+OK\r\n".to_vec()
}

fn expiry(args: &[String]) -> Option<i64> {
    args.get(3..)?
        .as_chunks::<2>()
        .0
        .iter()
        .find(|pair| {
            pair.first()
                .is_some_and(|option| option.eq_ignore_ascii_case("EX"))
        })
        .and_then(|pair| pair.get(1))
        .and_then(|value| value.parse::<i64>().ok())
}

fn lookup(entries: &Entries, command: &Command) -> Vec<u8> {
    command
        .args
        .get(1)
        .and_then(|key| locked(entries).get(key).map(|entry| entry.value.clone()))
        .map_or_else(|| b"$-1\r\n".to_vec(), |value| encode_bulk(&value))
}

fn encode_bulk(value: &str) -> Vec<u8> {
    let mut reply = format!("${}\r\n", value.len()).into_bytes();
    reply.extend_from_slice(value.as_bytes());
    reply.extend_from_slice(b"\r\n");
    reply
}

fn locked(registry: &Mutex<Registry>) -> MutexGuard<'_, Registry> {
    match registry.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
