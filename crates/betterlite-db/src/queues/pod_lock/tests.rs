#![cfg(test)]

use std::fs;
use std::future::{Future, pending, ready};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::future::BoxFuture;
use rstest::rstest;
use tokio::time::timeout;
use uuid::Uuid;

use super::owner::{PodId, PodOwnerError, decode_pod_owner, lease_key};
use super::{
    LockCache, LockCacheError, LockScript, NoPodLockEventSink, PodLockCallType, PodLockEvent,
    PodLockEventSink, PodLockManager,
};
use crate::queues::test_support::captured_logs;

// The JSON string B126's async_set_cache stores, the ARGV text the Lua compares, and the DEL bookkeeping of the fallback.
#[derive(Default)]
struct LeaseStore {
    stored_json: Option<String>,
    script_arguments: Vec<String>,
    lease: Option<String>,
    deleted_owners: Vec<String>,
}

#[derive(Clone, Copy)]
enum ScriptMode {
    Compare,
    Fail,
}

struct FakeCache {
    set: Mutex<Result<bool, ()>>,
    get: Mutex<Result<Option<Vec<u8>>, ()>>,
    delete: Mutex<Result<i64, ()>>,
    script: Mutex<Option<ScriptMode>>,
    store: Arc<Mutex<LeaseStore>>,
    registrations: AtomicUsize,
    deletes: AtomicUsize,
    script_runs: Arc<AtomicUsize>,
    set_ttls: Mutex<Vec<i64>>,
    stored_owners: Mutex<Vec<String>>,
    takeover: Mutex<Option<String>>,
    pending_delete: AtomicBool,
}

impl Default for FakeCache {
    fn default() -> Self {
        Self {
            set: Mutex::new(Ok(true)),
            get: Mutex::new(Ok(None)),
            delete: Mutex::new(Ok(0)),
            script: Mutex::new(None),
            store: Arc::default(),
            registrations: AtomicUsize::new(0),
            deletes: AtomicUsize::new(0),
            script_runs: Arc::new(AtomicUsize::new(0)),
            set_ttls: Mutex::new(Vec::new()),
            stored_owners: Mutex::new(Vec::new()),
            takeover: Mutex::new(None),
            pending_delete: AtomicBool::new(false),
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

    fn compare_script(&self) {
        *self.script.lock().unwrap_or_else(PoisonError::into_inner) = Some(ScriptMode::Compare);
    }

    fn failing_script(&self) {
        *self.script.lock().unwrap_or_else(PoisonError::into_inner) = Some(ScriptMode::Fail);
    }

    fn holds_lease(&self, owner: &str) {
        self.store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stored_json = Some(serde_json::to_string(owner).unwrap());
    }

    fn has_lease(&self) -> bool {
        self.store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stored_json
            .is_some()
    }

    fn held_owner(&self) -> Option<String> {
        self.store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .lease
            .clone()
    }

    fn script_arguments(&self) -> Vec<String> {
        self.store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .script_arguments
            .clone()
    }

    fn delete_count(&self, count: i64) {
        *self.delete.lock().unwrap_or_else(PoisonError::into_inner) = Ok(count);
    }

    fn delay_delete(&self) {
        self.pending_delete.store(true, Ordering::SeqCst);
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

    // the lease the fallback's DEL removes when the command runs
    fn lease_held_by(&self, owner: &str) {
        self.store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .lease = Some(owner.to_owned());
    }

    fn deleted_owners(&self) -> Vec<String> {
        self.store
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .deleted_owners
            .clone()
    }
}

impl LockCache for FakeCache {
    #[rustfmt::skip]
    fn get_owner(&self, _key: &str) -> impl Future<Output = Result<Option<Vec<u8>>, LockCacheError>> + Send {
        let guard = self.get.lock().unwrap_or_else(PoisonError::into_inner);
        let outcome = match &*guard {
            Ok(owner) => Ok(owner.clone()),
            Err(()) => Err(LockCacheError::UnexpectedReply { operation: "get_owner" }),
        };
        drop(guard);
        let replacement = self
            .takeover
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if matches!(&outcome, Ok(Some(_)))
            && let Some(next_owner) = replacement
        {
            self.store.lock().unwrap_or_else(PoisonError::into_inner).lease = Some(next_owner);
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
        let guard = self.set.lock().unwrap_or_else(PoisonError::into_inner);
        let outcome = match &*guard {
            Ok(stored) => Ok(*stored),
            Err(()) => Err(LockCacheError::UnexpectedReply { operation: "set_owner_if_absent" }),
        };
        drop(guard);
        if matches!(&outcome, Ok(true)) {
            self.store.lock().unwrap_or_else(PoisonError::into_inner).stored_json =
                Some(serde_json::to_string(owner).unwrap());
        }
        ready(outcome)
    }

    #[rustfmt::skip]
    fn delete(&self, _key: &str) -> impl Future<Output = Result<i64, LockCacheError>> + Send {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        let store = Arc::clone(&self.store);
        let delayed = self.pending_delete.load(Ordering::SeqCst);
        let guard = self.delete.lock().unwrap_or_else(PoisonError::into_inner);
        let outcome = match &*guard {
            Ok(count) => Ok(*count),
            Err(()) => Err(LockCacheError::UnexpectedReply { operation: "delete" }),
        };
        drop(guard);
        async move {
            if delayed {
                pending::<()>().await;
            }
            let mut lease = store.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(owner) = lease.lease.take() {
                lease.deleted_owners.push(owner);
            }
            drop(lease);
            outcome
        }
    }

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        self.registrations.fetch_add(1, Ordering::SeqCst);
        let mode = *self.script.lock().unwrap_or_else(PoisonError::into_inner);
        mode.map(|reply| -> Arc<dyn LockScript> {
            Arc::new(FakeScript {
                mode: reply,
                store: Arc::clone(&self.store),
                runs: Arc::clone(&self.script_runs),
            })
        })
    }
}

struct FakeScript {
    mode: ScriptMode,
    store: Arc<Mutex<LeaseStore>>,
    runs: Arc<AtomicUsize>,
}

impl LockScript for FakeScript {
    fn run<'lease>(
        &'lease self,
        _key: &'lease str,
        owner_json: &'lease str,
    ) -> BoxFuture<'lease, Result<bool, LockCacheError>> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            let mut lease = self.store.lock().unwrap_or_else(PoisonError::into_inner);
            lease.script_arguments.push(owner_json.to_owned());
            let result = match self.mode {
                ScriptMode::Fail => Err(LockCacheError::UnexpectedReply { operation: "run" }),
                ScriptMode::Compare if lease.stored_json.as_deref() == Some(owner_json) => {
                    lease.stored_json = None;
                    Ok(true)
                }
                ScriptMode::Compare => Ok(false),
            };
            drop(lease);
            result
        })
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
    let sink: Arc<dyn PodLockEventSink> = Arc::<RecordingEvents>::clone(events);
    PodLockManager::new(
        Some(Arc::clone(cache)),
        60,
        sink,
        PodId::try_from(Uuid::new_v4()).unwrap(),
    )
}

fn owner_text(manager: &PodLockManager<FakeCache>) -> &str {
    manager.pod_id.as_ref()
}

#[tokio::test]
async fn acquiring_without_a_cache_returns_none() {
    let manager = PodLockManager::<FakeCache>::new(
        None,
        60,
        Arc::new(NoPodLockEventSink),
        PodId::try_from(Uuid::new_v4()).unwrap(),
    );
    assert_eq!(manager.acquire_lock("job", None, true).await, None);
    manager.release_lock("job").await;
}

#[tokio::test]
async fn a_fresh_set_acquires_without_an_event() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    assert_eq!(manager.acquire_lock("job", None, true).await, Some(true));
    let owner = owner_text(&manager);
    assert_eq!(cache.stored_owners(), vec![owner.to_owned()]);
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
    cache.respond_with_owner(manager.pod_id.as_ref());
    assert_eq!(manager.acquire_lock("job", None, true).await, Some(true));
    // The reentrant path only reads: one failed NX SET, and no second SET that could push the expiry out.
    assert_eq!(cache.set_ttls(), vec![60]);
    assert_eq!(
        events.events(),
        vec![PodLockEvent {
            call_type: PodLockCallType::Acquired,
            gauge_labels: format!("job:{}", owner_text(&manager)),
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
    cache.respond_with_owner(manager.pod_id.as_ref());
    assert_eq!(
        manager.acquire_lock("job", Some(120), true).await,
        Some(true)
    );
    assert_eq!(
        events.events(),
        vec![PodLockEvent {
            call_type: PodLockCallType::Acquired,
            gauge_labels: format!("job:{}", owner_text(&manager)),
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
    cache.respond_with_owner(manager.pod_id.as_ref());
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
    cache.compare_script();
    assert_eq!(manager.acquire_lock("job", None, true).await, Some(true));
    manager.release_lock("job").await;
    manager.release_lock("job").await;
    assert_eq!(cache.registrations(), 1);
    assert_eq!(cache.script_runs(), 2);
    assert_eq!(cache.deletes(), 0);
    assert!(!cache.has_lease());
    assert_eq!(
        events.events(),
        vec![PodLockEvent {
            call_type: PodLockCallType::Released,
            gauge_labels: format!("job:{}", owner_text(&manager)),
            gauge_value: 0.0,
            duration_seconds: 60.0,
        }]
    );
}

#[tokio::test]
async fn the_release_script_compares_the_json_encoded_owner() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.compare_script();
    assert_eq!(manager.acquire_lock("job", None, true).await, Some(true));
    manager.release_lock("job").await;
    assert_eq!(
        cache.script_arguments(),
        vec![format!("\"{}\"", owner_text(&manager))]
    );
    assert!(!cache.has_lease());
    assert_eq!(events.events().len(), 1);
}

#[tokio::test]
async fn a_script_that_finds_another_owner_deletes_nothing() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.compare_script();
    cache.holds_lease("another-pod");
    manager.release_lock("job").await;
    assert_eq!(
        cache.script_arguments(),
        vec![format!("\"{}\"", owner_text(&manager))]
    );
    assert!(cache.has_lease());
    assert!(events.events().is_empty());
}

#[tokio::test]
async fn a_script_failure_clears_the_handle_warns_and_falls_back() {
    let (logs, _guard) = captured_logs();
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.failing_script();
    cache.respond_with_owner(manager.pod_id.as_ref());
    cache.delete_count(1);
    manager.release_lock("job").await;
    assert_eq!(cache.registrations(), 1);
    assert_eq!(cache.deletes(), 1);
    assert_eq!(
        events.events(),
        vec![PodLockEvent {
            call_type: PodLockCallType::Released,
            gauge_labels: format!("job:{}", owner_text(&manager)),
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
    cache.respond_with_owner(manager.pod_id.as_ref());
    cache.delete_count(1);
    cache.reacquired_between_get_and_delete("another-pod");
    manager.release_lock("job").await;
    assert_eq!(cache.deletes(), 1);
    assert_eq!(cache.deleted_owners(), vec!["another-pod".to_owned()]);
    assert_eq!(
        events.events(),
        vec![PodLockEvent {
            call_type: PodLockCallType::Released,
            gauge_labels: format!("job:{}", owner_text(&manager)),
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
    cache.respond_with_owner(manager.pod_id.as_ref());
    cache.delete_count(0);
    manager.release_lock("job").await;
    assert_eq!(cache.deletes(), 1);
    assert!(events.events().is_empty());
}

#[tokio::test]
async fn a_cancelled_release_keeps_the_lease_and_emits_nothing() {
    let cache = Arc::new(FakeCache::default());
    let events = Arc::new(RecordingEvents::default());
    let manager = build(&cache, &events);
    cache.respond_with_owner(manager.pod_id.as_ref());
    cache.lease_held_by(manager.pod_id.as_ref());
    cache.delete_count(1);
    cache.delay_delete();
    let cancelled = timeout(Duration::from_millis(20), manager.release_lock("job")).await;
    assert!(cancelled.is_err());
    assert_eq!(cache.deletes(), 1);
    assert!(cache.deleted_owners().is_empty());
    let owner = owner_text(&manager);
    assert_eq!(cache.held_owner(), Some(owner.to_owned()));
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
    let manager = PodLockManager::new(
        Some(Arc::clone(&cache)),
        60,
        sink,
        PodId::try_from(owner).unwrap(),
    );
    assert_eq!(owner_text(&manager), owner.to_string().as_str());
    assert_eq!(manager.acquire_lock("first", None, true).await, Some(true));
    assert_eq!(manager.acquire_lock("second", None, true).await, Some(true));
    assert_eq!(
        cache.stored_owners(),
        vec![owner.to_string(), owner.to_string()]
    );
}

#[rstest]
#[case::nil("00000000-0000-0000-0000-000000000000")]
#[case::v1("67e55044-10b1-126f-9247-bb680e5fe0c8")]
#[case::v3("67e55044-10b1-326f-9247-bb680e5fe0c8")]
#[case::v5("67e55044-10b1-526f-9247-bb680e5fe0c8")]
#[case::v7("67e55044-10b1-726f-9247-bb680e5fe0c8")]
#[case::wrong_variant("67e55044-10b1-426f-7247-bb680e5fe0c8")]
fn rejects_owners_that_are_not_v4(#[case] owner: &str) {
    assert!(matches!(
        PodId::try_from(Uuid::parse_str(owner).unwrap()),
        Err(PodOwnerError::NotV4)
    ));
}

#[test]
fn accepts_a_v4_owner() {
    let text = "67e55044-10b1-426f-9247-bb680e5fe0c8";
    let owner = PodId::try_from(Uuid::parse_str(text).unwrap()).unwrap();
    assert_eq!(owner.as_ref(), text);
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
    let unicode_job = format!("spend:reset-{}", char::from(0xe9));
    assert_eq!(
        lease_key(&unicode_job),
        format!("cronjob_lock:{unicode_job}")
    );
    assert_eq!(lease_key(""), "cronjob_lock:");
}

#[test]
fn decodes_utf8_owner_bytes_and_refuses_the_rest() {
    assert_eq!(decode_pod_owner(b"d3ad").unwrap(), "d3ad");
    let unicode_owner = char::from(0xe9).to_string();
    assert_eq!(
        decode_pod_owner(unicode_owner.as_bytes()).unwrap(),
        unicode_owner
    );
    assert!(decode_pod_owner(&[0xff, 0xfe]).is_err());
}

#[rstest]
fn replays_the_pod_owner_corpus(#[files("fuzz/corpus/pod_owner/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_pod_owner(&bytes));
}
