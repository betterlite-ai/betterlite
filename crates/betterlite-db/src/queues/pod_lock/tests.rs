#![cfg(test)]

use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rstest::rstest;
use uuid::Uuid;

use super::owner::{decode_pod_owner, lease_key};
use super::{
    LockCache, LockCacheError, LockScript, NoPodLockEventSink, PodLockCallType, PodLockEvent,
    PodLockEventSink, PodLockManager,
};
use crate::queues::test_support::captured_logs;

struct FakeCache {
    set: Mutex<Result<bool, ()>>,
    get: Mutex<Result<Option<Vec<u8>>, ()>>,
    delete: Mutex<Result<i64, ()>>,
    scripts: Mutex<Option<Result<bool, ()>>>,
    registrations: AtomicUsize,
    deletes: AtomicUsize,
    script_runs: Arc<AtomicUsize>,
    set_ttls: Mutex<Vec<i64>>,
    stored_owners: Mutex<Vec<String>>,
    lease: Mutex<Option<String>>,
    takeover: Mutex<Option<String>>,
    deleted_owners: Mutex<Vec<String>>,
}

impl Default for FakeCache {
    fn default() -> Self {
        Self {
            set: Mutex::new(Ok(true)),
            get: Mutex::new(Ok(None)),
            delete: Mutex::new(Ok(0)),
            scripts: Mutex::new(None),
            registrations: AtomicUsize::new(0),
            deletes: AtomicUsize::new(0),
            script_runs: Arc::new(AtomicUsize::new(0)),
            set_ttls: Mutex::new(Vec::new()),
            stored_owners: Mutex::new(Vec::new()),
            lease: Mutex::new(None),
            takeover: Mutex::new(None),
            deleted_owners: Mutex::new(Vec::new()),
        }
    }
}

impl FakeCache {
    fn respond_with_owner(&self, owner: &str) {
        *self.get.lock().unwrap_or_else(PoisonError::into_inner) =
            Ok(Some(owner.as_bytes().to_vec()));
    }

    fn contend(&self) {
        *self.set.lock().unwrap_or_else(PoisonError::into_inner) = Ok(false);
    }

    fn fail_set(&self) {
        *self.set.lock().unwrap_or_else(PoisonError::into_inner) = Err(());
    }

    fn fail_get(&self) {
        *self.get.lock().unwrap_or_else(PoisonError::into_inner) = Err(());
    }

    fn scripted(&self, result: bool) {
        *self.scripts.lock().unwrap_or_else(PoisonError::into_inner) = Some(Ok(result));
    }

    fn failing_script(&self) {
        *self.scripts.lock().unwrap_or_else(PoisonError::into_inner) = Some(Err(()));
    }

    fn delete_count(&self, count: i64) {
        *self.delete.lock().unwrap_or_else(PoisonError::into_inner) = Ok(count);
    }

    fn set_ttls(&self) -> Vec<i64> {
        self.set_ttls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn stored_owners(&self) -> Vec<String> {
        self.stored_owners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn registrations(&self) -> usize {
        self.registrations.load(Ordering::SeqCst)
    }

    fn deletes(&self) -> usize {
        self.deletes.load(Ordering::SeqCst)
    }

    fn script_runs(&self) -> usize {
        self.script_runs.load(Ordering::SeqCst)
    }

    // the fallback's GET reads our owner, then another pod's set lands before the fallback's DEL runs
    fn reacquired_between_get_and_delete(&self, owner: &str) {
        *self.takeover.lock().unwrap_or_else(PoisonError::into_inner) = Some(owner.to_owned());
    }

    fn deleted_owners(&self) -> Vec<String> {
        self.deleted_owners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl LockCache for FakeCache {
    #[rustfmt::skip]
    fn get_owner(&self, _key: &str) -> impl Future<Output = Result<Option<Vec<u8>>, LockCacheError>> + Send {
        let outcome = match &*self.get.lock().unwrap_or_else(PoisonError::into_inner) {
            Ok(owner) => Ok(owner.clone()),
            Err(()) => Err(LockCacheError::UnexpectedReply { operation: "get_owner" }),
        };
        if matches!(&outcome, Ok(Some(_)))
            && let Some(replacement) = self
                .takeover
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
        {
            *self.lease.lock().unwrap_or_else(PoisonError::into_inner) = Some(replacement);
        }
        ready(outcome)
    }

    #[rustfmt::skip]
    fn set_owner_if_absent(&self, _key: &str, owner: &str, ttl_seconds: i64) -> impl Future<Output = Result<bool, LockCacheError>> + Send {
        self.set_ttls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ttl_seconds);
        self.stored_owners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(owner.to_owned());
        let outcome = match &*self.set.lock().unwrap_or_else(PoisonError::into_inner) {
            Ok(stored) => Ok(*stored),
            Err(()) => Err(LockCacheError::UnexpectedReply { operation: "set_owner_if_absent" }),
        };
        ready(outcome)
    }

    #[rustfmt::skip]
    fn delete(&self, _key: &str) -> impl Future<Output = Result<i64, LockCacheError>> + Send {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        if let Some(owner) = self
            .lease
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            self.deleted_owners
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(owner);
        }
        let outcome = match &*self.delete.lock().unwrap_or_else(PoisonError::into_inner) {
            Ok(count) => Ok(*count),
            Err(()) => Err(LockCacheError::UnexpectedReply { operation: "delete" }),
        };
        ready(outcome)
    }

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        self.registrations.fetch_add(1, Ordering::SeqCst);
        let scripted = *self.scripts.lock().unwrap_or_else(PoisonError::into_inner);
        scripted.map(|result| -> Arc<dyn LockScript> {
            Arc::new(FakeScript {
                result,
                runs: Arc::clone(&self.script_runs),
            })
        })
    }
}

struct FakeScript {
    result: Result<bool, ()>,
    runs: Arc<AtomicUsize>,
}

impl LockScript for FakeScript {
    #[rustfmt::skip]
    fn run(&self, _key: &str, _owner: &str) -> impl Future<Output = Result<bool, LockCacheError>> + Send {
        self.runs.fetch_add(1, Ordering::SeqCst);
        let outcome = self
            .result
            .map_err(|()| LockCacheError::UnexpectedReply { operation: "run" });
        ready(outcome)
    }
}

#[derive(Default)]
struct RecordingEvents(Mutex<Vec<PodLockEvent>>);

impl PodLockEventSink for RecordingEvents {
    fn emit(&self, event: PodLockEvent) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

impl RecordingEvents {
    fn events(&self) -> Vec<PodLockEvent> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn build(cache: &Arc<FakeCache>, events: &Arc<RecordingEvents>) -> PodLockManager<FakeCache> {
    let sink: Arc<dyn PodLockEventSink> = Arc::clone(events);
    PodLockManager::new(Some(Arc::clone(cache)), 60, sink, Uuid::new_v4())
}

#[tokio::test]
async fn acquiring_without_a_cache_returns_none() {
    let manager =
        PodLockManager::<FakeCache>::new(None, 60, Arc::new(NoPodLockEventSink), Uuid::new_v4());
    assert_eq!(manager.acquire_lock("job", None, true).await, None);
    manager.release_lock("job").await;
}

#[tokio::test]
async fn a_fresh_set_acquires_without_an_event() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    assert_eq!(manager.acquire_lock("job", None, true).await, Some(true));
    assert_eq!(cache.stored_owners(), vec![manager.pod_id.clone()]);
    assert_eq!(cache.set_ttls(), vec![60]);
    assert!(events.events().is_empty());
}

#[tokio::test]
async fn a_truthy_lease_ttl_is_used_and_zero_falls_back() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    assert_eq!(
        manager.acquire_lock("job", Some(120), true).await,
        Some(true)
    );
    assert_eq!(manager.acquire_lock("job", Some(0), true).await, Some(true));
    assert_eq!(
        manager.acquire_lock("job", Some(-5), true).await,
        Some(true)
    );
    assert_eq!(cache.set_ttls(), vec![120, 60, -5]);
}

#[tokio::test]
async fn a_reentrant_acquire_succeeds_and_emits_the_acquired_gauge() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.contend();
    cache.respond_with_owner(&manager.pod_id);
    assert_eq!(manager.acquire_lock("job", None, true).await, Some(true));
    assert_eq!(
        events.events(),
        vec![PodLockEvent {
            call_type: PodLockCallType::Acquired,
            gauge_labels: format!("job:{}", manager.pod_id),
            gauge_value: 1.0,
            duration_seconds: 60.0,
        }]
    );
}

#[tokio::test]
async fn a_reentrant_event_reports_the_default_ttl_for_a_custom_lease() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.contend();
    cache.respond_with_owner(&manager.pod_id);
    assert_eq!(
        manager.acquire_lock("job", Some(120), true).await,
        Some(true)
    );
    assert_eq!(
        events.events(),
        vec![PodLockEvent {
            call_type: PodLockCallType::Acquired,
            gauge_labels: format!("job:{}", manager.pod_id),
            gauge_value: 1.0,
            duration_seconds: 60.0,
        }]
    );
}

#[tokio::test]
async fn reentrancy_can_be_disabled() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.contend();
    cache.respond_with_owner(&manager.pod_id);
    assert_eq!(manager.acquire_lock("job", None, false).await, Some(false));
    assert!(events.events().is_empty());
}

#[tokio::test]
async fn a_failed_set_held_by_another_pod_returns_false() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.contend();
    cache.respond_with_owner("another-pod");
    assert_eq!(manager.acquire_lock("job", None, true).await, Some(false));
    assert!(events.events().is_empty());
}

#[tokio::test]
async fn a_failed_set_with_a_missing_key_returns_false() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.contend();
    assert_eq!(manager.acquire_lock("job", None, true).await, Some(false));
    assert!(events.events().is_empty());
}

#[tokio::test]
async fn cache_errors_return_false_with_a_logged_diagnostic() {
    let (logs, _guard) = captured_logs();
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.fail_set();
    assert_eq!(manager.acquire_lock("job", None, true).await, Some(false));
    assert!(logs.text().contains("Error acquiring Redis lock"));
    assert!(events.events().is_empty());
}

#[tokio::test]
async fn an_owner_that_is_not_utf8_returns_false() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.contend();
    *cache.get.lock().unwrap_or_else(PoisonError::into_inner) = Ok(Some(vec![0xff, 0xfe]));
    assert_eq!(manager.acquire_lock("job", None, true).await, Some(false));
    assert!(events.events().is_empty());
}

#[tokio::test]
async fn release_uses_the_cached_script_and_emits_only_for_one() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.scripted(true);
    manager.release_lock("job").await;
    manager.release_lock("job").await;
    assert_eq!(cache.registrations(), 1);
    assert_eq!(cache.script_runs(), 2);
    assert_eq!(cache.deletes(), 0);
    assert_eq!(
        events.events(),
        vec![
            PodLockEvent {
                call_type: PodLockCallType::Released,
                gauge_labels: format!("job:{}", manager.pod_id),
                gauge_value: 0.0,
                duration_seconds: 60.0,
            },
            PodLockEvent {
                call_type: PodLockCallType::Released,
                gauge_labels: format!("job:{}", manager.pod_id),
                gauge_value: 0.0,
                duration_seconds: 60.0,
            }
        ]
    );
}

#[tokio::test]
async fn a_script_that_deletes_nothing_emits_nothing() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.scripted(false);
    manager.release_lock("job").await;
    assert!(events.events().is_empty());
}

#[tokio::test]
async fn a_script_failure_clears_the_handle_warns_and_falls_back() {
    let (logs, _guard) = captured_logs();
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.failing_script();
    cache.respond_with_owner(&manager.pod_id);
    cache.delete_count(1);
    manager.release_lock("job").await;
    assert_eq!(cache.registrations(), 1);
    assert_eq!(cache.deletes(), 1);
    assert_eq!(
        events.events(),
        vec![PodLockEvent {
            call_type: PodLockCallType::Released,
            gauge_labels: format!("job:{}", manager.pod_id),
            gauge_value: 0.0,
            duration_seconds: 60.0,
        }]
    );
    assert!(
        logs.text()
            .contains("Lua compare-and-delete failed for lock_key=cronjob_lock:job")
    );
    manager.release_lock("job").await;
    assert_eq!(cache.registrations(), 2);
}

#[tokio::test]
async fn the_get_and_delete_fallback_cannot_be_atomic() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    // GET returns our owner, another pod's set lands before DEL, and the fallback deletes that fresh lease.
    cache.respond_with_owner(&manager.pod_id);
    cache.delete_count(1);
    cache.reacquired_between_get_and_delete("another-pod");
    manager.release_lock("job").await;
    assert_eq!(cache.deletes(), 1);
    assert_eq!(cache.deleted_owners(), vec!["another-pod".to_owned()]);
    assert_eq!(
        events.events(),
        vec![PodLockEvent {
            call_type: PodLockCallType::Released,
            gauge_labels: format!("job:{}", manager.pod_id),
            gauge_value: 0.0,
            duration_seconds: 60.0,
        }]
    );
}

#[tokio::test]
async fn the_fallback_keeps_another_pods_lock() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.respond_with_owner("another-pod");
    cache.delete_count(1);
    manager.release_lock("job").await;
    assert_eq!(cache.deletes(), 0);
    assert!(events.events().is_empty());
}

#[tokio::test]
async fn a_fallback_delete_that_removes_nothing_emits_nothing() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.respond_with_owner(&manager.pod_id);
    cache.delete_count(0);
    manager.release_lock("job").await;
    assert_eq!(cache.deletes(), 1);
    assert!(events.events().is_empty());
}

#[tokio::test]
async fn release_errors_are_logged_and_swallowed() {
    let (logs, _guard) = captured_logs();
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.fail_get();
    manager.release_lock("job").await;
    assert!(logs.text().contains("Error releasing Redis lock"));
    assert!(events.events().is_empty());
}

#[tokio::test]
async fn the_constructed_uuid_owns_every_acquire() {
    let owner = Uuid::new_v4();
    let cache = Arc::new(FakeCache::default());
    let sink: Arc<dyn PodLockEventSink> = Arc::new(NoPodLockEventSink);
    let manager = PodLockManager::new(Some(Arc::clone(&cache)), 60, sink, owner);
    assert_eq!(manager.pod_id, owner.to_string());
    manager.acquire_lock("first", None, true).await;
    manager.acquire_lock("second", None, true).await;
    assert_eq!(
        cache.stored_owners(),
        vec![owner.to_string(), owner.to_string()]
    );
}

#[test]
fn call_types_carry_the_service_logger_names() {
    assert_eq!(
        PodLockCallType::Acquired.as_str(),
        "_emit_acquired_lock_event"
    );
    assert_eq!(
        PodLockCallType::Released.as_str(),
        "_emit_released_lock_event"
    );
}

#[test]
fn keys_keep_colons_and_unicode_in_the_job_name() {
    assert_eq!(lease_key("spend:reset-é"), "cronjob_lock:spend:reset-é");
    assert_eq!(lease_key(""), "cronjob_lock:");
}

#[test]
fn decodes_utf8_owner_bytes_and_refuses_the_rest() {
    assert_eq!(decode_pod_owner(b"d3ad").unwrap(), "d3ad");
    assert_eq!(decode_pod_owner("é".as_bytes()).unwrap(), "é");
    assert!(decode_pod_owner(&[0xff, 0xfe]).is_err());
}

#[rstest]
fn replays_the_pod_owner_corpus(#[files("fuzz/corpus/pod_owner/*")] path: std::path::PathBuf) {
    let bytes = std::fs::read(path).unwrap();
    let _owner = decode_pod_owner(&bytes);
}
