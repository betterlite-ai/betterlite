#![cfg(test)]

use std::future::pending;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::runtime::{Builder, Runtime};

use super::{DEFAULT_MAX_SIZE_IN_MEMORY, DEFAULT_TTL_SECONDS, LlmClientCache, event_loop_key};
use crate::evicted_client_closer::{
    CLOSE_GRACE_SECONDS, Clock, CloseMethod, EvictableClient, EvictedClientCloser,
    MAX_PENDING_CLOSES, current_event_loop_id,
};

#[path = "tests/logging_clients.rs"]
mod logging_clients;
#[path = "tests/service_trace_ids.rs"]
mod service_trace_ids;

struct ProbeClient {
    close_method: CloseMethod,
    closes: AtomicUsize,
    async_closes: AtomicUsize,
}

impl ProbeClient {
    fn new(close_method: CloseMethod) -> Arc<Self> {
        Arc::new(Self {
            close_method,
            closes: AtomicUsize::new(0),
            async_closes: AtomicUsize::new(0),
        })
    }

    fn sync() -> Arc<Self> {
        Self::new(CloseMethod::Sync)
    }

    fn asynchronous() -> Arc<Self> {
        Self::new(CloseMethod::Async)
    }
}

#[async_trait]
impl EvictableClient for ProbeClient {
    fn close_method(&self) -> Option<CloseMethod> {
        Some(self.close_method)
    }

    fn close(&self) {
        self.closes.fetch_add(1, Ordering::SeqCst);
    }

    async fn close_async(&self) {
        self.async_closes.fetch_add(1, Ordering::SeqCst);
    }
}

struct HangingClient;

#[async_trait]
impl EvictableClient for HangingClient {
    fn close_method(&self) -> Option<CloseMethod> {
        Some(CloseMethod::Async)
    }

    async fn close_async(&self) {
        pending::<()>().await;
    }
}

type TestTime = Arc<Mutex<f64>>;

fn fake_clock(time: &TestTime) -> Clock {
    let shared = Arc::clone(time);
    Arc::new(move || *shared.lock().expect("fixture clock lock is not poisoned"))
}

fn advance(time: &TestTime, seconds: f64) {
    let mut now = time.lock().expect("fixture clock lock is not poisoned");
    *now = seconds.mul_add(1.0_f64, *now);
}

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("the test runtime is built")
}

struct Fixture {
    time: TestTime,
    closer: Arc<EvictedClientCloser<ProbeClient>>,
    cache: LlmClientCache<ProbeClient>,
}

fn fixture() -> Fixture {
    let time = Arc::new(Mutex::new(0.0_f64));
    let closer = Arc::new(EvictedClientCloser::new(fake_clock(&time)));
    let cache = LlmClientCache::new(
        DEFAULT_MAX_SIZE_IN_MEMORY,
        DEFAULT_TTL_SECONDS,
        fake_clock(&time),
        Arc::clone(&closer),
    )
    .expect("the fixture cache is built");
    Fixture {
        time,
        closer,
        cache,
    }
}

fn close_count(client: &ProbeClient) -> usize {
    client.closes.load(Ordering::SeqCst)
}

fn async_close_count(client: &ProbeClient) -> usize {
    client.async_closes.load(Ordering::SeqCst)
}

#[test]
fn llm_cache_event_loop_key_appends_the_running_loop_id() {
    let running = runtime();
    let decorated = running.block_on(async { event_loop_key("httpx_client") });
    let loop_id = running
        .block_on(async { current_event_loop_id() })
        .expect("the runtime is running");

    assert_eq!(decorated, format!("httpx_client-{loop_id}"));
}

#[test]
fn llm_cache_event_loop_key_without_a_loop_is_unchanged() {
    assert_eq!(event_loop_key("httpx_client"), "httpx_client");
}

#[test]
fn llm_cache_serves_a_client_inside_the_loop_that_stored_it() {
    let fixture = fixture();
    let client = ProbeClient::sync();
    let running = runtime();
    running.block_on(async {
        fixture
            .cache
            .set_cache("httpx_client", Arc::clone(&client), None, false)
            .expect("set succeeds");
        let cached = fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds");
        assert!(cached.is_some_and(|found| Arc::ptr_eq(&found, &client)));
    });
}

#[test]
fn llm_cache_misses_under_a_different_loop() {
    let fixture = fixture();
    let client = ProbeClient::sync();
    let storing = runtime();
    let other = runtime();
    storing.block_on(async {
        fixture
            .cache
            .set_cache("httpx_client", Arc::clone(&client), None, false)
            .expect("set succeeds");
    });

    other.block_on(async {
        assert!(
            fixture
                .cache
                .get_cache("httpx_client")
                .expect("get succeeds")
                .is_none()
        );
    });
    assert!(
        fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_none()
    );
    storing.block_on(async {
        assert!(
            fixture
                .cache
                .get_cache("httpx_client")
                .expect("get succeeds")
                .is_some()
        );
    });
}

#[test]
fn llm_cache_without_a_running_loop_keeps_the_plain_key() {
    let fixture = fixture();
    fixture
        .cache
        .set_cache("httpx_client", ProbeClient::sync(), None, false)
        .expect("set succeeds");

    assert!(
        fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_some()
    );
}

#[test]
fn llm_cache_keeps_two_live_clients_under_different_keys() {
    let fixture = fixture();
    fixture
        .cache
        .set_cache("first", ProbeClient::sync(), None, false)
        .expect("set succeeds");
    fixture
        .cache
        .set_cache("second", ProbeClient::sync(), None, false)
        .expect("set succeeds");

    assert!(
        fixture
            .cache
            .get_cache("first")
            .expect("get succeeds")
            .is_some()
    );
    assert!(
        fixture
            .cache
            .get_cache("second")
            .expect("get succeeds")
            .is_some()
    );
}

#[test]
fn llm_cache_set_evicts_and_schedules_an_expired_client() {
    let fixture = fixture();
    fixture
        .cache
        .set_cache("expired", ProbeClient::sync(), Some(1.0_f64), true)
        .expect("set succeeds");

    advance(&fixture.time, 2.0_f64);
    fixture
        .cache
        .set_cache("fresh", ProbeClient::sync(), None, false)
        .expect("set succeeds");

    assert_eq!(fixture.closer.pending_count(), 1);
    assert!(
        fixture
            .cache
            .get_cache("expired")
            .expect("get succeeds")
            .is_none()
    );
}

#[test]
fn llm_cache_set_closes_a_due_client_without_a_read() {
    let fixture = fixture();
    let early = ProbeClient::sync();
    let late = ProbeClient::sync();
    fixture
        .cache
        .set_cache("early", Arc::clone(&early), Some(1.0_f64), true)
        .expect("set succeeds");
    fixture
        .cache
        .set_cache("late", Arc::clone(&late), Some(901.0_f64), true)
        .expect("set succeeds");

    advance(&fixture.time, 2.0_f64);
    fixture
        .cache
        .set_cache("fresh-one", ProbeClient::sync(), None, false)
        .expect("set succeeds");

    advance(&fixture.time, 900.0_f64);
    fixture
        .cache
        .set_cache("fresh-two", ProbeClient::sync(), None, false)
        .expect("set succeeds");

    assert_eq!(close_count(&early), 1);
    assert_eq!(close_count(&late), 0);
    assert_eq!(fixture.closer.pending_count(), 1);
}

#[test]
fn llm_cache_default_ttl_is_six_hundred_seconds() {
    let fixture = fixture();
    fixture
        .cache
        .set_cache("httpx_client", ProbeClient::sync(), None, false)
        .expect("set succeeds");

    advance(&fixture.time, 599.5_f64);
    assert!(
        fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_some()
    );

    advance(&fixture.time, 0.5_f64);
    assert!(
        fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_some()
    );

    advance(&fixture.time, 0.5_f64);
    assert!(
        fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_none()
    );
}

#[test]
fn llm_cache_honours_a_ttl_override() {
    let fixture = fixture();
    fixture
        .cache
        .set_cache("httpx_client", ProbeClient::sync(), Some(3600.0_f64), false)
        .expect("set succeeds");

    advance(&fixture.time, 600.0_f64);
    assert!(
        fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_some()
    );

    advance(&fixture.time, 3000.0_f64);
    assert!(
        fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_some()
    );

    advance(&fixture.time, 0.5_f64);
    assert!(
        fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_none()
    );
}

#[test]
fn llm_cache_uses_the_configured_default_ttl() {
    let time: TestTime = Arc::new(Mutex::new(0.0_f64));
    let closer = Arc::new(EvictedClientCloser::new(fake_clock(&time)));
    let cache = LlmClientCache::new(
        DEFAULT_MAX_SIZE_IN_MEMORY,
        10.0_f64,
        fake_clock(&time),
        closer,
    )
    .expect("the fixture cache is built");
    cache
        .set_cache("httpx_client", ProbeClient::sync(), None, false)
        .expect("set succeeds");

    advance(&time, 9.5_f64);
    assert!(
        cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_some()
    );

    advance(&time, 0.5_f64);
    assert!(
        cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_some()
    );

    advance(&time, 0.5_f64);
    assert!(
        cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_none()
    );
}

fn fill_two_hundred_entries(cache: &LlmClientCache<ProbeClient>) {
    (0..200_u32).for_each(|index| {
        cache
            .set_cache(
                &format!("client-{index:03}"),
                ProbeClient::sync(),
                None,
                false,
            )
            .expect("set succeeds");
    });
}

#[test]
fn llm_cache_evicts_the_earliest_expiry_after_two_hundred_clients() {
    let fixture = fixture();
    fill_two_hundred_entries(&fixture.cache);

    assert!(
        fixture
            .cache
            .get_cache("client-000")
            .expect("get succeeds")
            .is_some()
    );

    fixture
        .cache
        .set_cache("client-200", ProbeClient::sync(), None, false)
        .expect("set succeeds");

    assert!(
        fixture
            .cache
            .get_cache("client-000")
            .expect("get succeeds")
            .is_none()
    );
    assert!(
        fixture
            .cache
            .get_cache("client-001")
            .expect("get succeeds")
            .is_some()
    );
    assert!(
        fixture
            .cache
            .get_cache("client-200")
            .expect("get succeeds")
            .is_some()
    );
}

#[test]
fn llm_cache_capacity_zero_stores_nothing() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let closer = Arc::new(EvictedClientCloser::new(fake_clock(&time)));
    let cache = LlmClientCache::new(0, DEFAULT_TTL_SECONDS, fake_clock(&time), closer)
        .expect("the fixture cache is built");

    cache
        .set_cache("httpx_client", ProbeClient::sync(), None, true)
        .expect("set succeeds");

    assert!(
        cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_none()
    );
}

#[test]
fn llm_cache_never_closes_a_caller_supplied_client() {
    let fixture = fixture();
    let client = ProbeClient::sync();
    fixture
        .cache
        .set_cache("httpx_client", Arc::clone(&client), Some(1.0_f64), false)
        .expect("set succeeds");

    advance(&fixture.time, 2.0_f64);
    assert!(
        fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_none()
    );

    advance(&fixture.time, 900.0_f64);
    assert!(
        fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_none()
    );

    assert_eq!(close_count(&client), 0);
}

#[test]
fn llm_cache_closes_a_litellm_owned_client_after_the_grace_window() {
    let fixture = fixture();
    let client = ProbeClient::sync();
    fixture
        .cache
        .set_cache("httpx_client", Arc::clone(&client), Some(1.0_f64), true)
        .expect("set succeeds");

    advance(&fixture.time, 2.0_f64);
    assert!(
        fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_none()
    );
    assert_eq!(fixture.closer.pending_count(), 1);

    advance(&fixture.time, 899.5_f64);
    assert!(
        fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_none()
    );
    assert_eq!(close_count(&client), 0);

    advance(&fixture.time, 0.5_f64);
    assert!(
        fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_none()
    );
    assert_eq!(close_count(&client), 1);
}

#[test]
fn llm_cache_keeps_a_live_entry_expiry_when_overwritten() {
    let fixture = fixture();
    fixture
        .cache
        .set_cache("httpx_client", ProbeClient::sync(), None, false)
        .expect("set succeeds");

    advance(&fixture.time, 60.0_f64);
    fixture
        .cache
        .set_cache("httpx_client", ProbeClient::sync(), None, false)
        .expect("set succeeds");

    advance(&fixture.time, 540.0_f64);
    assert!(
        fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_some()
    );

    advance(&fixture.time, 0.5_f64);
    assert!(
        fixture
            .cache
            .get_cache("httpx_client")
            .expect("get succeeds")
            .is_none()
    );
}

#[test]
fn llm_cache_same_key_set_after_expiry_retires_the_old_client() {
    let fixture = fixture();
    let old = ProbeClient::sync();
    fixture
        .cache
        .set_cache("httpx_client", Arc::clone(&old), Some(1.0_f64), true)
        .expect("set succeeds");

    advance(&fixture.time, 2.0_f64);
    fixture
        .cache
        .set_cache("httpx_client", ProbeClient::sync(), None, true)
        .expect("set succeeds");
    assert_eq!(fixture.closer.pending_count(), 1);

    advance(&fixture.time, 900.0_f64);
    fixture
        .cache
        .set_cache("another", ProbeClient::sync(), None, false)
        .expect("set succeeds");
    assert_eq!(close_count(&old), 1);
}

#[test]
fn llm_cache_capacity_victim_same_key_retires_the_old_client() {
    let fixture = fixture();
    fill_two_hundred_entries(&fixture.cache);

    let old = ProbeClient::sync();
    fixture
        .cache
        .set_cache("client-000", Arc::clone(&old), None, true)
        .expect("set succeeds");
    assert_eq!(fixture.closer.pending_count(), 0);

    fixture
        .cache
        .set_cache("client-000", ProbeClient::sync(), None, true)
        .expect("set succeeds");
    assert_eq!(fixture.closer.pending_count(), 1);

    advance(&fixture.time, 900.0_f64);
    fixture
        .cache
        .set_cache("another", ProbeClient::sync(), None, false)
        .expect("set succeeds");
    assert_eq!(close_count(&old), 1);
}

#[test]
fn llm_cache_live_overwrite_does_not_retire_the_old_client() {
    let fixture = fixture();
    let old = ProbeClient::sync();
    fixture
        .cache
        .set_cache("httpx_client", Arc::clone(&old), None, true)
        .expect("set succeeds");

    advance(&fixture.time, 60.0_f64);
    fixture
        .cache
        .set_cache("httpx_client", ProbeClient::sync(), None, true)
        .expect("set succeeds");
    assert_eq!(fixture.closer.pending_count(), 0);

    advance(&fixture.time, 900.0_f64);
    fixture
        .cache
        .set_cache("another", ProbeClient::sync(), None, false)
        .expect("set succeeds");
    assert_eq!(close_count(&old), 0);
}

#[test]
fn llm_cache_async_get_closes_a_due_async_client() {
    let fixture = fixture();
    let client = ProbeClient::asynchronous();
    let running = runtime();
    running.block_on(async {
        fixture
            .cache
            .async_set_cache("httpx_client", Arc::clone(&client), Some(1.0_f64), true)
            .await
            .expect("set succeeds");

        advance(&fixture.time, 2.0_f64);
        assert!(
            fixture
                .cache
                .async_get_cache("httpx_client")
                .await
                .expect("get succeeds")
                .is_none()
        );

        advance(&fixture.time, 900.0_f64);
        assert!(
            fixture
                .cache
                .async_get_cache("httpx_client")
                .await
                .expect("get succeeds")
                .is_none()
        );
        assert_eq!(async_close_count(&client), 1);
    });
}

#[test]
fn llm_cache_async_get_returns_when_an_async_close_hangs() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let closer = Arc::new(EvictedClientCloser::with_limits(
        CLOSE_GRACE_SECONDS,
        MAX_PENDING_CLOSES,
        Duration::from_millis(10),
        fake_clock(&time),
    ));
    let cache = LlmClientCache::new(
        DEFAULT_MAX_SIZE_IN_MEMORY,
        DEFAULT_TTL_SECONDS,
        fake_clock(&time),
        closer,
    )
    .expect("the fixture cache is built");
    let client = Arc::new(HangingClient);
    let running = runtime();
    running.block_on(async {
        cache
            .async_set_cache("httpx_client", Arc::clone(&client), Some(1.0_f64), true)
            .await
            .expect("set succeeds");

        advance(&time, 2.0_f64);
        assert!(
            cache
                .async_get_cache("httpx_client")
                .await
                .expect("get succeeds")
                .is_none()
        );

        advance(&time, 900.0_f64);
        assert!(
            cache
                .async_get_cache("httpx_client")
                .await
                .expect("get succeeds")
                .is_none()
        );
    });
}
