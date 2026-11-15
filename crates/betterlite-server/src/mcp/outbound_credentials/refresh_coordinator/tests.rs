#![cfg(test)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::time::{sleep, timeout};

use super::{RedisRefreshCoordinator, RedisTunables};
use crate::mcp::outbound_credentials::distributed_lock::{DistributedLock, LockAcquisition};
use crate::mcp::outbound_credentials::{RefreshCoordinator as _, RefreshStep, TokenKey};

const KEY: &str = "mcp:refresh_lock:user-1:server-1";

struct FakeLock {
    acquisitions: Mutex<VecDeque<LockAcquisition>>,
    held: Mutex<VecDeque<bool>>,
    extends: Mutex<VecDeque<bool>>,
    events: Mutex<Vec<String>>,
}

impl FakeLock {
    fn new(acquisitions: Vec<LockAcquisition>) -> Arc<Self> {
        Arc::new(Self {
            acquisitions: Mutex::new(VecDeque::from(acquisitions)),
            held: Mutex::new(VecDeque::new()),
            extends: Mutex::new(VecDeque::new()),
            events: Mutex::new(Vec::new()),
        })
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }

    fn record(&self, event: String) {
        self.events.lock().unwrap().push(event);
    }
}

#[async_trait]
impl DistributedLock for Arc<FakeLock> {
    async fn acquire(&self, key: &str, token: &str, ttl: Duration) -> LockAcquisition {
        self.record(format!("acquire {key} {token} {ttl:?}"));
        self.acquisitions
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(LockAcquisition::Held)
    }

    async fn extend(&self, key: &str, token: &str, ttl: Duration) -> bool {
        self.record(format!("extend {key} {token} {ttl:?}"));
        self.extends.lock().unwrap().pop_front().unwrap_or(false)
    }

    async fn release(&self, key: &str, token: &str) {
        self.record(format!("release {key} {token}"));
    }

    async fn is_held(&self, key: &str) -> bool {
        self.record(format!("is_held {key}"));
        self.held.lock().unwrap().pop_front().unwrap_or(false)
    }
}

fn key() -> TokenKey {
    TokenKey {
        user_id: "user-1".to_owned(),
        server_id: "server-1".to_owned(),
    }
}

fn tunables() -> RedisTunables {
    RedisTunables {
        key_prefix: "mcp:refresh_lock:".to_owned(),
        lock_ttl: Duration::from_millis(100),
        refresh_budget: Duration::from_secs(10),
        wait_timeout: Duration::from_millis(200),
        poll_interval: Duration::from_millis(50),
    }
}

fn coordinator(
    lock: Arc<FakeLock>,
    tunables: RedisTunables,
) -> RedisRefreshCoordinator<Arc<FakeLock>, String, String> {
    let mut coordinator = RedisRefreshCoordinator::new(lock);
    coordinator.tunables = tunables;
    coordinator
}

fn counted(value: &'static str, calls: Arc<AtomicUsize>) -> RefreshStep<'static, String, String> {
    Box::new(move || {
        calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(value.to_owned()) })
    })
}

fn delayed(delay: Duration) -> RefreshStep<'static, String, String> {
    Box::new(move || {
        Box::pin(async move {
            sleep(delay).await;
            Ok("fresh".to_owned())
        })
    })
}

fn reread() -> RefreshStep<'static, String, String> {
    Box::new(|| Box::pin(async { Ok("reread".to_owned()) }))
}

#[tokio::test(start_paused = true)]
async fn an_acquired_lock_runs_the_refresh_and_releases() {
    let lock = FakeLock::new(vec![LockAcquisition::Acquired]);
    let calls = Arc::new(AtomicUsize::new(0));
    let result = coordinator(Arc::clone(&lock), tunables())
        .run(key(), counted("fresh", Arc::clone(&calls)), reread())
        .await;
    assert_eq!(result, Ok("fresh".to_owned()));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let events = lock.events();
    assert_eq!(events.len(), 2);
    let parts: Vec<&str> = events[0].split_whitespace().collect();
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[0], "acquire");
    assert_eq!(parts[1], KEY);
    assert_eq!(parts[3], "100ms");
    let token = parts[2];
    assert_eq!(token.len(), 32);
    assert!(
        token
            .chars()
            .all(|letter| letter.is_ascii_hexdigit() && !letter.is_ascii_uppercase())
    );
    assert_eq!(events[1], format!("release {KEY} {token}"));
}

#[tokio::test(start_paused = true)]
async fn an_error_on_acquire_refreshes_without_the_lock() {
    let lock = FakeLock::new(vec![LockAcquisition::Error]);
    let calls = Arc::new(AtomicUsize::new(0));
    let result = coordinator(Arc::clone(&lock), tunables())
        .run(key(), counted("fresh", Arc::clone(&calls)), reread())
        .await;
    assert_eq!(result, Ok("fresh".to_owned()));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(lock.events().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_held_lock_waits_until_the_holder_lets_go_then_rereads() {
    let lock = FakeLock::new(vec![LockAcquisition::Held]);
    lock.held.lock().unwrap().extend([true, false]);
    let calls = Arc::new(AtomicUsize::new(0));
    let result = coordinator(Arc::clone(&lock), tunables())
        .run(key(), counted("fresh", Arc::clone(&calls)), reread())
        .await;
    assert_eq!(result, Ok("reread".to_owned()));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let events = lock.events();
    assert_eq!(events.len(), 3);
    assert!(events[0].starts_with(&format!("acquire {KEY} ")));
    assert_eq!(events[1], format!("is_held {KEY}"));
    assert_eq!(events[2], format!("is_held {KEY}"));
}

#[tokio::test(start_paused = true)]
async fn a_held_lock_stops_waiting_at_the_timeout() {
    let lock = FakeLock::new(vec![LockAcquisition::Held]);
    lock.held
        .lock()
        .unwrap()
        .extend([true, true, true, true, true]);
    let result = coordinator(Arc::clone(&lock), tunables())
        .run(
            key(),
            counted("fresh", Arc::new(AtomicUsize::new(0))),
            reread(),
        )
        .await;
    assert_eq!(result, Ok("reread".to_owned()));
    let polls = lock
        .events()
        .iter()
        .filter(|event| event.starts_with("is_held "))
        .count();
    assert_eq!(polls, 4);
}

#[tokio::test(start_paused = true)]
async fn a_slow_refresh_renews_the_lease_each_half_ttl() {
    let lock = FakeLock::new(vec![LockAcquisition::Acquired]);
    lock.extends.lock().unwrap().extend([true, true, true]);
    let result = coordinator(Arc::clone(&lock), tunables())
        .run(key(), delayed(Duration::from_millis(120)), reread())
        .await;
    assert_eq!(result, Ok("fresh".to_owned()));
    let events = lock.events();
    let renewals = events
        .iter()
        .filter(|event| event.starts_with("extend "))
        .count();
    assert_eq!(renewals, 2);
    assert!(events.last().unwrap().starts_with("release "));
}

#[tokio::test(start_paused = true)]
async fn a_lost_renewal_stops_renewing_and_finishes_the_refresh() {
    let lock = FakeLock::new(vec![LockAcquisition::Acquired]);
    lock.extends.lock().unwrap().extend([false, true]);
    let result = coordinator(Arc::clone(&lock), tunables())
        .run(key(), delayed(Duration::from_millis(400)), reread())
        .await;
    assert_eq!(result, Ok("fresh".to_owned()));
    let events = lock.events();
    let renewals = events
        .iter()
        .filter(|event| event.starts_with("extend "))
        .count();
    assert_eq!(renewals, 1);
    assert!(events.last().unwrap().starts_with("release "));
}

#[tokio::test(start_paused = true)]
async fn the_renewal_stops_at_the_refresh_budget() {
    let mut tunables = tunables();
    tunables.refresh_budget = Duration::from_millis(120);
    let lock = FakeLock::new(vec![LockAcquisition::Acquired]);
    lock.extends
        .lock()
        .unwrap()
        .extend([true, true, true, true]);
    let result = coordinator(Arc::clone(&lock), tunables)
        .run(key(), delayed(Duration::from_secs(1)), reread())
        .await;
    assert_eq!(result, Ok("fresh".to_owned()));
    let events = lock.events();
    let renewals = events
        .iter()
        .filter(|event| event.starts_with("extend "))
        .count();
    assert_eq!(renewals, 3);
    assert!(events.last().unwrap().starts_with("release "));
}

#[tokio::test(start_paused = true)]
async fn a_zero_budget_never_renews_the_lease() {
    let mut tunables = tunables();
    tunables.refresh_budget = Duration::ZERO;
    let lock = FakeLock::new(vec![LockAcquisition::Acquired]);
    lock.extends.lock().unwrap().extend([true, true]);
    let result = coordinator(Arc::clone(&lock), tunables)
        .run(key(), delayed(Duration::from_millis(120)), reread())
        .await;
    assert_eq!(result, Ok("fresh".to_owned()));
    let events = lock.events();
    assert_eq!(events.len(), 2);
    assert!(events[1].starts_with("release "));
}

#[tokio::test(start_paused = true)]
async fn a_failed_refresh_is_returned_and_still_releases() {
    let lock = FakeLock::new(vec![LockAcquisition::Acquired]);
    let flight: RefreshStep<'static, String, String> =
        Box::new(|| Box::pin(async { Err("endpoint down".to_owned()) }));
    let result = coordinator(Arc::clone(&lock), tunables())
        .run(key(), flight, reread())
        .await;
    assert_eq!(result, Err("endpoint down".to_owned()));
    assert!(lock.events().last().unwrap().starts_with("release "));
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_run_leaves_the_lease_to_expire() {
    let lock = FakeLock::new(vec![LockAcquisition::Acquired]);
    let coordinator = coordinator(Arc::clone(&lock), tunables());
    let run = coordinator.run(key(), delayed(Duration::from_secs(30)), reread());
    let expired = timeout(Duration::from_millis(20), run).await;
    let _elapsed = expired.unwrap_err();
    let events = lock.events();
    assert_eq!(events.len(), 1);
    assert!(events[0].starts_with("acquire "));
}

#[tokio::test(start_paused = true)]
async fn the_key_prefix_and_lock_ttl_come_from_the_tunables() {
    let mut custom = tunables();
    custom.key_prefix = "custom:".to_owned();
    custom.lock_ttl = Duration::from_millis(1500);
    let lock = FakeLock::new(vec![LockAcquisition::Acquired]);
    coordinator(Arc::clone(&lock), custom)
        .run(
            key(),
            counted("fresh", Arc::new(AtomicUsize::new(0))),
            reread(),
        )
        .await
        .unwrap();
    let events = lock.events();
    assert!(events[0].starts_with("acquire custom:user-1:server-1 "));
    assert!(events[0].ends_with(" 1.5s"));
}

#[test]
fn the_default_tunables_match_litellm() {
    let tunables = RedisTunables::default();
    assert_eq!(tunables.key_prefix, "mcp:refresh_lock:");
    assert_eq!(tunables.lock_ttl, Duration::from_secs(10));
    assert_eq!(tunables.refresh_budget, Duration::from_secs(20));
    assert_eq!(tunables.wait_timeout, Duration::from_secs(35));
    assert_eq!(tunables.poll_interval, Duration::from_millis(50));
}
