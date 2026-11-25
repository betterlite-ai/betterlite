#![cfg(test)]

use std::future::pending;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_redis::breaker_metrics::BreakerMetrics;
use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_router::redis_failure_policy::RedisFailurePolicy;
use betterlite_router::redis_failure_policy::Settings as BreakerSettings;
use betterlite_router::redis_failure_policy::gate::PolicyGate;
use betterlite_router::routing_state_store::BreakerClock;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3_async_runtimes::tokio::get_runtime;

use super::*;
use crate::cache_backends::settings::RedisSettings;

// The numeric overrides are text because arbitrary_precision breaks the untagged number fields. Nothing listens on port 1, so a call reaches the client and every command fails on the way out.
const DEAD_REDIS: &str = r#"{"host": "127.0.0.1", "port": "1", "socket_timeout": "0.1", "socket_connect_timeout": "0.1"}"#;
// Short enough that a test crosses the recovery timeout, long enough that an ungated call stays inside it.
const RECOVERY_TIMEOUT: Duration = Duration::from_secs(10);

struct FakeClock {
    millis: AtomicU64,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            millis: AtomicU64::new(0),
        }
    }

    fn handle(self: &Arc<Self>) -> BreakerClock {
        let clock = Arc::clone(self);
        Arc::new(move || Duration::from_millis(clock.millis.load(Ordering::Relaxed)))
    }

    fn advance(&self, by: Duration) {
        let step = u64::try_from(by.as_millis()).unwrap_or(u64::MAX);
        self.millis.fetch_add(step, Ordering::Relaxed);
    }
}

fn breaker_policy(threshold: u32, enabled: bool) -> RedisFailurePolicy {
    RedisFailurePolicy::new(
        BreakerSettings {
            enabled,
            failure_threshold: threshold,
            recovery_timeout: RECOVERY_TIMEOUT,
            timeout_min_duration: Duration::from_secs(5),
            timeout_log_interval: Duration::from_secs(5),
        },
        BreakerMetrics::new(None),
    )
}

fn gate_with(policy: RedisFailurePolicy, clock: &Arc<FakeClock>) -> Arc<PolicyGate> {
    Arc::new(PolicyGate::new(policy, clock.handle()))
}

fn dead_settings() -> RedisSettings {
    RedisSettings {
        overrides: serde_json::from_str(DEAD_REDIS).unwrap(),
        namespace: None,
        default_ttl: None,
        redis_flush_size: None,
    }
}

fn dead_redis() -> NativeRedis {
    Python::initialize();
    Python::attach(|py| open(py, dead_settings(), test_gate()).unwrap())
}

#[test]
fn backends_redis_errors_surface() {
    let native = dead_redis();
    Python::attach(|py| {
        native.set(py, "key", "1", None).unwrap();
        assert_eq!(native.get(py, "key").unwrap(), None);
        native.increment(py, "key", 1_i64, None).unwrap_err();
        let failed = native.delete(py, vec!["key".to_owned()]).unwrap_err();
        assert!(failed.is_instance_of::<PyRuntimeError>(py));
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
        let native = Python::attach(|py| open(py, settings, test_gate()).unwrap());
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
    let settings = dead_settings();
    let mut native = Python::attach(|py| open(py, settings, test_gate()).unwrap());
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
    let settings = dead_settings();
    Python::attach(|py| open(py, settings, test_gate()).unwrap());
}

#[test]
fn backends_redis_answers_the_value_text() {
    assert_eq!(
        answer_text(Some(CachedValue::Null)).unwrap(),
        Some("null".to_owned())
    );
    assert_eq!(answer_text(None).unwrap(), None);
}

// The threshold is one, so the call after the first failure never reaches Redis.
#[test]
fn backends_redis_breaker_opens_and_probes() {
    Python::initialize();
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(breaker_policy(1, true), &clock);
    let native = Python::attach(|py| open(py, dead_settings(), gate).unwrap());
    Python::attach(|py| {
        // the values slice swallows the connection failure after the breaker counted it
        assert_eq!(native.get(py, "key").unwrap(), None);
        let open = native.get(py, "key").unwrap_err();
        assert!(open.to_string().contains("circuit breaker is open"));
        clock.advance(RECOVERY_TIMEOUT + Duration::from_secs(1));
        // the probe is admitted and fails on the connection, so it answers like the first call
        assert_eq!(native.get(py, "key").unwrap(), None);
    });
}

// LiteLLM counts one breaker per RedisCache, so a namespace change keeps the open one.
#[test]
fn backends_redis_breaker_survives_a_namespace_change() {
    Python::initialize();
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(breaker_policy(1, true), &clock);
    let mut native = Python::attach(|py| open(py, dead_settings(), gate).unwrap());
    Python::attach(|py| {
        assert_eq!(native.get(py, "key").unwrap(), None);
        native.set_namespace(py, Some("other")).unwrap();
        let open = native.get(py, "key").unwrap_err();
        assert!(open.to_string().contains("circuit breaker is open"));
    });
}

// With the breaker off, every call reaches Redis and fails on the connection.
#[test]
fn backends_redis_breaker_disabled_never_opens() {
    Python::initialize();
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(breaker_policy(1, false), &clock);
    let native = Python::attach(|py| open(py, dead_settings(), gate).unwrap());
    for _call in 0..3 {
        let failed = get_runtime()
            .block_on(count(
                Arc::clone(&native.cache),
                "key".to_owned(),
                1.0_f64,
                None,
            ))
            .unwrap_err();
        assert!(!failed.to_string().contains("circuit breaker is open"));
    }
}

// The constructor reads the settings from the environment, as LiteLLM reads its constants.
#[test]
fn backends_redis_breaker_reads_the_environment() {
    Python::initialize();
    temp_env::with_vars(
        [("REDIS_CIRCUIT_BREAKER_FAILURE_THRESHOLD", Some("1"))],
        || {
            Python::attach(|py| {
                let native = NativeRedis::new(py, DEAD_REDIS).unwrap();
                assert_eq!(native.get(py, "key").unwrap(), None);
                let open = native.get(py, "key").unwrap_err();
                assert!(open.to_string().contains("circuit breaker is open"));
            });
        },
    );
}

#[test]
fn backends_redis_breaker_rejects_a_bad_setting() {
    Python::initialize();
    temp_env::with_vars(
        [("REDIS_CIRCUIT_BREAKER_FAILURE_THRESHOLD", Some("many"))],
        || {
            Python::attach(|py| {
                let error = NativeRedis::new(py, DEAD_REDIS).err().unwrap();
                assert!(error.is_instance_of::<PyValueError>(py));
                let message = error.to_string();
                assert!(message.contains("REDIS_CIRCUIT_BREAKER_FAILURE_THRESHOLD"));
            });
        },
    );
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
