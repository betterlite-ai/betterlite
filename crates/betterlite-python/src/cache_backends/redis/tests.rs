#![cfg(test)]

use std::collections::BTreeMap;
use std::future::pending;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use betterlite_cache::configuration::settings::{
    DEFAULT_REDIS_MAJOR_VERSION, DEFAULT_REDIS_TTL_SECONDS,
};
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::RedisCacheOptions;
use betterlite_redis::redis_cache::connect::connect;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use pyo3_async_runtimes::tokio::get_runtime;

use super::*;
use crate::cache_backends::settings::RedisSettings;

// The numeric overrides are text because arbitrary_precision breaks the untagged number fields. Nothing listens on port 1, so a call reaches the client and every command fails on the way out.
const DEAD_REDIS: &str = r#"{"host": "127.0.0.1", "port": "1", "socket_timeout": "0.1", "socket_connect_timeout": "0.1"}"#;

fn dead_redis() -> NativeRedis {
    Python::initialize();
    let overrides: RedisOverrides = serde_json::from_str(DEAD_REDIS).unwrap();
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: DEFAULT_REDIS_TTL_SECONDS,
        redis_flush_size: None,
        default_major_version: DEFAULT_REDIS_MAJOR_VERSION,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(QuietHooks),
        gate: Arc::new(QuietGate),
    };
    let environment = BTreeMap::<String, String>::new();
    let cache: RedisCache = get_runtime()
        .block_on(connect(options, &environment))
        .unwrap();
    NativeRedis {
        cache: Arc::new(cache),
        settings: RedisSettings::default(),
        tasks: TaskTracker::new(),
    }
}

#[test]
fn backends_redis_errors_surface() {
    let native = dead_redis();
    Python::attach(|py| {
        native.set(py, "key", "1", None).unwrap();
        assert_eq!(native.get(py, "key").unwrap(), None);
        native.increment(py, "key", 1_i64, None).unwrap_err();
        native.delete(py, vec!["key".to_owned()]).unwrap_err();
        native.flush(py).unwrap_err();
        native.ping(py).unwrap_err();
        assert_eq!(native.ttl(py, "key").unwrap(), None);
    });
}

#[test]
fn backends_redis_close_is_ok() {
    Python::initialize();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::scope(|scope| {
        let server = scope.spawn(move || serves_until_closed(&listener));
        let overrides: RedisOverrides = serde_json::from_str(&format!(
            r#"{{"host":"127.0.0.1","port":"{}"}}"#,
            address.port()
        ))
        .unwrap();
        let settings = RedisSettings {
            overrides,
            namespace: None,
            default_ttl: None,
            redis_flush_size: None,
        };
        let native = Python::attach(|py| open(py, settings).unwrap());
        drop(
            get_runtime()
                .block_on(native.cache.transport.async_client())
                .unwrap(),
        );
        Python::attach(|py| native.close(py)).unwrap();
        assert!(server.join().unwrap());
    });
}

fn serves_until_closed(listener: &TcpListener) -> bool {
    // Construction probes a sync connection before close releases the separate async connection.
    let (mut probe, _probe_peer) = listener.accept().unwrap();
    assert!(answer_handshake(&mut probe, true));
    drop(probe);
    let (mut client, _client_peer) = listener.accept().unwrap();
    answer_handshake(&mut client, false)
}

fn answer_handshake(stream: &mut TcpStream, probe: bool) -> bool {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return true,
            Err(_timeout) => return false,
            Ok(_read) => {}
        }
        let count: usize = line.trim().strip_prefix('*').unwrap().parse().unwrap();
        let mut command = Vec::new();
        for _field in 0..count {
            line.clear();
            reader.read_line(&mut line).unwrap();
            let size: usize = line.trim().strip_prefix('$').unwrap().parse().unwrap();
            let mut field = vec![0_u8; size];
            reader.read_exact(&mut field).unwrap();
            let mut terminator = [0_u8; 2];
            reader.read_exact(&mut terminator).unwrap();
            assert_eq!(terminator, *b"\r\n");
            command.push(field);
        }
        let reply: &[u8] = match command.first().map(Vec::as_slice) {
            Some(b"HELLO") => b"%1\r\n+proto\r\n:3\r\n",
            Some(b"INFO") => b"$21\r\nredis_version:7.2.0\r\n\r\n",
            Some(b"CLIENT") => b"+OK\r\n",
            Some(b"PING") => b"+PONG\r\n",
            other => panic!("unexpected Redis command {other:?}"),
        };
        stream.write_all(reply).unwrap();
        if probe && command.first().map(Vec::as_slice) == Some(b"PING") {
            return true;
        }
    }
}

// An equal namespace keeps the open client; a rebuilt one would swap the Arc the cache lives in.
#[test]
fn backends_redis_namespace_keeps_the_cache() {
    let mut native = dead_redis();
    let before = Arc::clone(&native.cache);
    Python::attach(|py| native.set_namespace(py, None).unwrap());
    assert!(Arc::ptr_eq(&before, &native.cache));
}

// A new namespace opens a new client, and the flushes the old one still runs stay in the tracker close drains.
#[test]
fn backends_redis_namespace_change_keeps_the_tasks() {
    Python::initialize();
    let overrides: RedisOverrides = serde_json::from_str(DEAD_REDIS).unwrap();
    let settings = RedisSettings {
        overrides,
        namespace: None,
        default_ttl: None,
        redis_flush_size: None,
    };
    let mut native = Python::attach(|py| open(py, settings).unwrap());
    let before = Arc::clone(&native.cache);
    let running = native.tasks.track_future(pending::<()>());
    Python::attach(|py| native.set_namespace(py, Some("other")).unwrap());
    assert!(!Arc::ptr_eq(&before, &native.cache));
    assert_eq!(native.tasks.len(), 1_usize);
    drop(running);
    assert!(native.tasks.is_empty());
}

// The constructor helper builds the same lazy client the other tests assemble by hand.
#[test]
fn backends_redis_open_connects() {
    let overrides: RedisOverrides = serde_json::from_str(DEAD_REDIS).unwrap();
    let settings = RedisSettings {
        overrides,
        namespace: None,
        default_ttl: None,
        redis_flush_size: None,
    };
    Python::attach(|py| open(py, settings).unwrap());
}

#[test]
fn backends_redis_answers_the_value_text() {
    assert_eq!(
        answer_text(Some(CachedValue::Null)).unwrap(),
        Some("null".to_owned())
    );
    assert_eq!(answer_text(None).unwrap(), None);
}

#[test]
fn backends_redis_gate_admits() {
    let gate = QuietGate;
    gate.enter("probe").unwrap();
}

#[test]
fn backends_redis_store_swallows_a_failure() {
    let native = dead_redis();
    get_runtime()
        .block_on(store(
            Arc::clone(&native.cache),
            "key".to_owned(),
            Json::Bool(true),
            None,
            false,
        ))
        .unwrap();
}

#[test]
fn backends_redis_count_reports_failure() {
    let native = dead_redis();
    get_runtime()
        .block_on(count(
            Arc::clone(&native.cache),
            "count".to_owned(),
            2.0_f64,
            None,
        ))
        .unwrap_err();
}
