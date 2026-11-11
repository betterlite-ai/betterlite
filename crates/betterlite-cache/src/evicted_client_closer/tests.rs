#![cfg(test)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::runtime::{Builder, Runtime};

use super::{
    CLOSE_GRACE_SECONDS, CLOSE_TIMEOUT, Clock, CloseMethod, EvictableClient, EvictedClientCloser,
    MAX_PENDING_CLOSES,
};

struct ProbeClient {
    close_method: Option<CloseMethod>,
    in_flight: AtomicBool,
    closes: AtomicUsize,
    async_closes: AtomicUsize,
}

impl ProbeClient {
    fn new(close_method: Option<CloseMethod>) -> Arc<Self> {
        Arc::new(Self {
            close_method,
            in_flight: AtomicBool::new(false),
            closes: AtomicUsize::new(0),
            async_closes: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl EvictableClient for ProbeClient {
    fn close_method(&self) -> Option<CloseMethod> {
        self.close_method
    }

    fn has_connection_in_flight(&self) -> bool {
        self.in_flight.load(Ordering::SeqCst)
    }

    fn close(&self) {
        self.closes.fetch_add(1, Ordering::SeqCst);
    }

    async fn close_async(&self) {
        self.async_closes.fetch_add(1, Ordering::SeqCst);
    }
}

struct SilentClient {
    closes: AtomicUsize,
}

#[async_trait]
impl EvictableClient for SilentClient {
    fn close_method(&self) -> Option<CloseMethod> {
        Some(CloseMethod::Sync)
    }

    fn close(&self) {
        self.closes.fetch_add(1, Ordering::SeqCst);
    }
}

type TestTime = Arc<Mutex<f64>>;

fn test_time() -> TestTime {
    Arc::new(Mutex::new(0.0_f64))
}

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

fn close_count(client: &ProbeClient) -> usize {
    client.closes.load(Ordering::SeqCst)
}

fn async_close_count(client: &ProbeClient) -> usize {
    client.async_closes.load(Ordering::SeqCst)
}

fn owned_count(closer: &EvictedClientCloser<ProbeClient>) -> usize {
    closer
        .owned
        .lock()
        .expect("fixture owned lock is not poisoned")
        .len()
}

#[test]
fn owning_a_collected_client_does_not_keep_it_in_the_owned_list() {
    let time = test_time();
    let closer = EvictedClientCloser::new(fake_clock(&time));
    let collected = ProbeClient::new(Some(CloseMethod::Sync));
    closer.mark_owned(&collected);
    assert_eq!(owned_count(&closer), 1);

    drop(collected);
    let live = ProbeClient::new(Some(CloseMethod::Sync));
    closer.mark_owned(&live);
    assert_eq!(owned_count(&closer), 1);

    let other = ProbeClient::new(Some(CloseMethod::Sync));
    closer.mark_owned(&other);
    assert_eq!(owned_count(&closer), 2);
}

#[test]
fn ownership_is_required_before_a_client_is_scheduled() {
    let time = test_time();
    let closer = EvictedClientCloser::new(fake_clock(&time));
    let client = ProbeClient::new(Some(CloseMethod::Sync));

    closer.schedule(&client);
    advance(&time, CLOSE_GRACE_SECONDS);
    closer.reap();

    assert_eq!(closer.pending_count(), 0);
    assert_eq!(close_count(&client), 0);
}

#[test]
fn an_owned_client_is_closed_only_once_the_grace_window_has_passed() {
    let time = test_time();
    let closer = EvictedClientCloser::new(fake_clock(&time));
    let client = ProbeClient::new(Some(CloseMethod::Sync));
    closer.mark_owned(&client);
    closer.schedule(&client);

    advance(&time, 899.5_f64);
    closer.reap();
    assert_eq!(close_count(&client), 0);

    advance(&time, 0.5_f64);
    closer.reap();
    assert_eq!(close_count(&client), 1);
    assert_eq!(closer.pending_count(), 0);
}

#[test]
fn a_client_with_a_connection_in_flight_is_requeued_until_it_is_idle() {
    let time = test_time();
    let closer = EvictedClientCloser::new(fake_clock(&time));
    let client = ProbeClient::new(Some(CloseMethod::Sync));
    closer.mark_owned(&client);
    client.in_flight.store(true, Ordering::SeqCst);
    closer.schedule(&client);

    advance(&time, 900.0_f64);
    closer.reap();
    assert_eq!(close_count(&client), 0);
    assert_eq!(closer.pending_count(), 1);

    client.in_flight.store(false, Ordering::SeqCst);
    advance(&time, 899.5_f64);
    closer.reap();
    assert_eq!(close_count(&client), 0);

    advance(&time, 0.5_f64);
    closer.reap();
    assert_eq!(close_count(&client), 1);
}

#[test]
fn idle_connection_state_is_assumed_for_clients_that_cannot_report_it() {
    let time = test_time();
    let closer = EvictedClientCloser::new(fake_clock(&time));
    let client = Arc::new(SilentClient {
        closes: AtomicUsize::new(0),
    });
    closer.mark_owned(&client);
    closer.schedule(&client);

    advance(&time, CLOSE_GRACE_SECONDS);
    closer.reap();

    assert_eq!(client.closes.load(Ordering::SeqCst), 1);
}

#[test]
fn bucket_keeps_clients_ordered_by_deadline() {
    let time = test_time();
    let closer = EvictedClientCloser::new(fake_clock(&time));
    let early = ProbeClient::new(Some(CloseMethod::Sync));
    let late = ProbeClient::new(Some(CloseMethod::Sync));
    closer.mark_owned(&early);
    closer.mark_owned(&late);
    closer.schedule(&early);

    advance(&time, 100.0_f64);
    closer.schedule(&late);
    advance(&time, 800.0_f64);
    closer.reap();

    assert_eq!(close_count(&early), 1);
    assert_eq!(close_count(&late), 0);

    advance(&time, 100.0_f64);
    closer.reap();
    assert_eq!(close_count(&late), 1);
}

#[test]
fn a_client_without_a_close_method_is_never_queued() {
    let time = test_time();
    let closer = EvictedClientCloser::new(fake_clock(&time));
    let client = ProbeClient::new(None);
    closer.mark_owned(&client);
    closer.schedule(&client);

    assert_eq!(closer.pending_count(), 0);
}

#[test]
fn queue_capacity_holds_ten_thousand_pending_closes() {
    let time = test_time();
    let closer = EvictedClientCloser::new(fake_clock(&time));
    let client = ProbeClient::new(Some(CloseMethod::Sync));
    closer.mark_owned(&client);
    (0..MAX_PENDING_CLOSES).for_each(|_index| closer.schedule(&client));

    assert_eq!(closer.pending_count(), MAX_PENDING_CLOSES);

    closer.schedule(&client);
    assert_eq!(closer.pending_count(), MAX_PENDING_CLOSES);
}

#[test]
fn a_custom_pending_limit_caps_the_queue() {
    let time = test_time();
    let closer = EvictedClientCloser::with_limits(900.0_f64, 2, CLOSE_TIMEOUT, fake_clock(&time));
    let client = ProbeClient::new(Some(CloseMethod::Sync));
    closer.mark_owned(&client);
    (0..3_u32).for_each(|_index| closer.schedule(&client));

    assert_eq!(closer.pending_count(), 2);
}

#[test]
fn a_custom_grace_window_sets_the_close_deadline() {
    let time = test_time();
    let closer = EvictedClientCloser::with_limits(10.0_f64, 100, CLOSE_TIMEOUT, fake_clock(&time));
    let client = ProbeClient::new(Some(CloseMethod::Sync));
    closer.mark_owned(&client);
    closer.schedule(&client);

    advance(&time, 9.5_f64);
    closer.reap();
    assert_eq!(close_count(&client), 0);

    advance(&time, 0.5_f64);
    closer.reap();
    assert_eq!(close_count(&client), 1);
}

#[test]
fn pending_count_drops_a_client_that_was_collected_before_the_enqueue() {
    let time = test_time();
    let closer = EvictedClientCloser::new(fake_clock(&time));
    let collected = ProbeClient::new(Some(CloseMethod::Sync));
    closer.mark_owned(&collected);
    closer.schedule(&collected);
    drop(collected);

    assert_eq!(closer.pending_count(), 1);

    let live = ProbeClient::new(Some(CloseMethod::Sync));
    closer.mark_owned(&live);
    closer.schedule(&live);

    assert_eq!(closer.pending_count(), 1);
}

#[test]
fn pending_count_drops_entries_whose_client_was_dropped_before_the_reap() {
    let time = test_time();
    let closer = EvictedClientCloser::new(fake_clock(&time));
    let collected = ProbeClient::new(Some(CloseMethod::Sync));
    closer.mark_owned(&collected);
    closer.schedule(&collected);
    drop(collected);

    advance(&time, CLOSE_GRACE_SECONDS);
    closer.reap();

    assert_eq!(closer.pending_count(), 0);
}

#[test]
fn async_close_waits_for_the_event_loop_it_was_scheduled_on() {
    let time = test_time();
    let closer = Arc::new(EvictedClientCloser::new(fake_clock(&time)));
    let client = ProbeClient::new(Some(CloseMethod::Async));
    closer.mark_owned(&client);
    let scheduling = runtime();
    let other = runtime();
    scheduling.block_on(async {
        closer.schedule(&client);
    });

    advance(&time, CLOSE_GRACE_SECONDS);
    closer.reap();
    assert_eq!(async_close_count(&client), 0);

    other.block_on(async {
        closer.reap_async().await;
    });
    assert_eq!(async_close_count(&client), 0);

    scheduling.block_on(async {
        closer.reap_async().await;
    });
    assert_eq!(async_close_count(&client), 1);
}

#[test]
fn async_client_scheduled_without_a_loop_closes_on_any_loop() {
    let time = test_time();
    let closer = EvictedClientCloser::new(fake_clock(&time));
    let client = ProbeClient::new(Some(CloseMethod::Async));
    closer.mark_owned(&client);
    closer.schedule(&client);

    advance(&time, CLOSE_GRACE_SECONDS);
    let reaping = runtime();
    reaping.block_on(async {
        closer.reap_async().await;
    });

    assert_eq!(async_close_count(&client), 1);
}

#[test]
fn sync_reap_leaves_async_clients_for_a_later_loop_reap() {
    let time = test_time();
    let closer = Arc::new(EvictedClientCloser::new(fake_clock(&time)));
    let client = ProbeClient::new(Some(CloseMethod::Async));
    closer.mark_owned(&client);
    let scheduling = runtime();
    scheduling.block_on(async {
        closer.schedule(&client);
    });

    advance(&time, CLOSE_GRACE_SECONDS);
    closer.reap();

    assert_eq!(async_close_count(&client), 0);
    assert_eq!(closer.pending_count(), 1);

    scheduling.block_on(async {
        closer.reap_async().await;
    });
    assert_eq!(async_close_count(&client), 1);
}
