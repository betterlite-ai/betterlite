#![cfg(test)]

use std::net::TcpStream;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_db::queues::pod_lock::cache::RedisLockCache;
use betterlite_db::queues::pod_lock::owner::PodId;
use betterlite_db::queues::pod_lock::{
    PodLockCallType, PodLockEvent, PodLockEventSink, PodLockManager,
};
use betterlite_logging::logging_service_events::sink::{ServiceEventError, ServiceEventSink};
use betterlite_logging::logging_service_events::types::{
    ServiceEvent, ServiceEventMetadata, ServiceLoggerPayload, ServiceType,
};
use betterlite_logging::logging_service_events::worker_link::Delivery;
use betterlite_logging::registry::CallbackRegistry;
use betterlite_logging::worker::{
    DEFAULT_EVENT_TIMEOUT, DEFAULT_SHUTDOWN_DRAIN, DEFAULT_TIMEOUT_SUMMARY_WINDOW, LoggingWorker,
    WorkerSettings,
};
use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_cache::RedisCache;
use chrono::Utc;
use redis::Value;
use tokio::task::JoinSet;
use uuid::Uuid;

use super::OracleRedis;

const LEASE_KEY: &str = "cronjob_lock:job";

async fn start_fixture() -> OracleRedis {
    let mut startup = JoinSet::new();
    startup.spawn_blocking(OracleRedis::start);
    startup.join_next().await.unwrap().unwrap().unwrap()
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

fn manager(
    cache: &Arc<RedisCache>,
    events: &Arc<RecordingEvents>,
) -> PodLockManager<RedisLockCache> {
    let sink: Arc<dyn PodLockEventSink> = Arc::<RecordingEvents>::clone(events);
    PodLockManager::new(
        Some(Arc::new(RedisLockCache {
            cache: Arc::clone(cache),
        })),
        60,
        sink,
        PodId::try_from(Uuid::new_v4()).unwrap(),
    )
}

#[tokio::test]
async fn a_first_acquire_stores_the_quoted_owner_with_an_expiry() {
    let fixture = start_fixture().await;
    let cache = fixture.cache(None).await.unwrap();
    let events = Arc::new(RecordingEvents::default());
    let holder = manager(&cache, &events);
    assert_eq!(holder.acquire_lock("job", None, true).await, Some(true));
    assert_eq!(
        cache.async_get_cache(LEASE_KEY).await.unwrap(),
        Some(CachedValue::Text(holder.pod_id.to_string()))
    );
    assert!(
        cache
            .async_get_ttl(LEASE_KEY)
            .await
            .unwrap()
            .is_some_and(|ttl| ttl > 0 && ttl <= 60)
    );
    assert!(events.events().is_empty());
}

#[tokio::test]
async fn another_pod_is_denied_while_the_lease_lives() {
    let fixture = start_fixture().await;
    let cache = fixture.cache(None).await.unwrap();
    let holder_events = Arc::new(RecordingEvents::default());
    let holder = manager(&cache, &holder_events);
    let other_events = Arc::new(RecordingEvents::default());
    let other = manager(&cache, &other_events);
    assert_eq!(holder.acquire_lock("job", Some(30), true).await, Some(true));
    assert_eq!(other.acquire_lock("job", None, true).await, Some(false));
    assert!(other_events.events().is_empty());
    assert_eq!(
        cache.async_get_cache(LEASE_KEY).await.unwrap(),
        Some(CachedValue::Text(holder.pod_id.to_string()))
    );
}

#[tokio::test]
async fn simultaneous_nx_acquisitions_have_one_owner() {
    let fixture = start_fixture().await;
    let cache = fixture.cache(None).await.unwrap();
    let first = manager(&cache, &Arc::new(RecordingEvents::default()));
    let second = manager(&cache, &Arc::new(RecordingEvents::default()));
    let (first_result, second_result) = tokio::join!(
        first.acquire_lock("job", None, true),
        second.acquire_lock("job", None, true),
    );
    assert_ne!(first_result, second_result);
    assert!(matches!(first_result, Some(true) | Some(false)));
    assert!(matches!(second_result, Some(true) | Some(false)));
    let owner = if first_result == Some(true) {
        first.pod_id
    } else {
        second.pod_id
    };
    assert_eq!(
        cache.async_get_cache(LEASE_KEY).await.unwrap(),
        Some(CachedValue::Text(owner.to_string()))
    );
}

#[tokio::test]
async fn redis_stores_the_json_encoded_owner() {
    let fixture = start_fixture().await;
    let cache = fixture.cache(None).await.unwrap();
    let holder = manager(&cache, &Arc::new(RecordingEvents::default()));
    assert_eq!(holder.acquire_lock("job", None, true).await, Some(true));
    let script = cache
        .counters
        .async_register_script("return redis.call('get', KEYS[1])");
    let stored = cache
        .run_script(&script, &[LEASE_KEY.to_owned()], &[])
        .await
        .unwrap();
    assert_eq!(
        stored,
        Value::BulkString(serde_json::to_vec(holder.pod_id.as_ref()).unwrap())
    );
}

#[tokio::test]
async fn a_reentrant_acquire_keeps_the_original_expiry_and_emits_once() {
    let fixture = start_fixture().await;
    let cache = fixture.cache(None).await.unwrap();
    let events = Arc::new(RecordingEvents::default());
    let holder = manager(&cache, &events);
    assert_eq!(holder.acquire_lock("job", Some(2), true).await, Some(true));
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(holder.acquire_lock("job", Some(2), true).await, Some(true));
    assert!(
        cache
            .async_get_ttl(LEASE_KEY)
            .await
            .unwrap()
            .is_some_and(|ttl| ttl <= 1)
    );
    assert_eq!(
        events.events(),
        vec![PodLockEvent {
            call_type: PodLockCallType::Acquired,
            gauge_labels: format!("job:{}", holder.pod_id),
            gauge_value: 1.0,
            duration_seconds: 60.0,
        }]
    );
}

#[tokio::test]
async fn an_expired_lease_is_reacquired_by_another_pod() {
    let fixture = start_fixture().await;
    let cache = fixture.cache(None).await.unwrap();
    let first_events = Arc::new(RecordingEvents::default());
    let first = manager(&cache, &first_events);
    let second_events = Arc::new(RecordingEvents::default());
    let second = manager(&cache, &second_events);
    assert_eq!(first.acquire_lock("job", Some(1), true).await, Some(true));
    assert_eq!(second.acquire_lock("job", None, true).await, Some(false));
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(second.acquire_lock("job", None, true).await, Some(true));
    assert_eq!(
        cache.async_get_cache(LEASE_KEY).await.unwrap(),
        Some(CachedValue::Text(second.pod_id.to_string()))
    );
    assert!(second_events.events().is_empty());
}

#[tokio::test]
async fn a_stale_owner_cannot_release_a_reacquired_lease() {
    let fixture = start_fixture().await;
    let cache = fixture.cache(None).await.unwrap();
    let stale_events = Arc::new(RecordingEvents::default());
    let stale = manager(&cache, &stale_events);
    let live_events = Arc::new(RecordingEvents::default());
    let live = manager(&cache, &live_events);
    assert_eq!(stale.acquire_lock("job", Some(1), true).await, Some(true));
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(live.acquire_lock("job", None, true).await, Some(true));
    stale.release_lock("job").await;
    assert!(stale_events.events().is_empty());
    assert_eq!(
        cache.async_get_cache(LEASE_KEY).await.unwrap(),
        Some(CachedValue::Text(live.pod_id.to_string()))
    );
    assert_eq!(live.acquire_lock("job", None, true).await, Some(true));
    assert_eq!(
        live_events.events(),
        vec![PodLockEvent {
            call_type: PodLockCallType::Acquired,
            gauge_labels: format!("job:{}", live.pod_id),
            gauge_value: 1.0,
            duration_seconds: 60.0,
        }]
    );
}

#[tokio::test]
async fn the_fixture_runs_the_loaded_source_not_a_rust_copy() {
    let fixture = start_fixture().await;
    let cache = fixture.cache(None).await.unwrap();
    let events = Arc::new(RecordingEvents::default());
    let holder = manager(&cache, &events);
    assert_eq!(holder.acquire_lock("job", None, true).await, Some(true));
    // EVALSHA runs the source SCRIPT LOAD received, so a script that deletes unconditionally deletes the live lease.
    let script = cache
        .counters
        .async_register_script("return redis.call(\"del\", KEYS[1])");
    let deleted = cache
        .run_script(&script, &[LEASE_KEY.to_owned()], &[])
        .await
        .unwrap();
    assert_eq!(deleted, Value::Int(1));
    assert_eq!(cache.async_get_cache(LEASE_KEY).await.unwrap(), None);
}

#[tokio::test]
async fn release_deletes_the_lease_and_emits_one_event() {
    let fixture = start_fixture().await;
    let cache = fixture.cache(None).await.unwrap();
    let events = Arc::new(RecordingEvents::default());
    let holder = manager(&cache, &events);
    assert_eq!(holder.acquire_lock("job", None, true).await, Some(true));
    holder.release_lock("job").await;
    assert_eq!(cache.async_get_cache(LEASE_KEY).await.unwrap(), None);
    assert_eq!(
        events.events(),
        vec![PodLockEvent {
            call_type: PodLockCallType::Released,
            gauge_labels: format!("job:{}", holder.pod_id),
            gauge_value: 0.0,
            duration_seconds: 60.0,
        }]
    );
    holder.release_lock("job").await;
    assert_eq!(
        events.events(),
        vec![PodLockEvent {
            call_type: PodLockCallType::Released,
            gauge_labels: format!("job:{}", holder.pod_id),
            gauge_value: 0.0,
            duration_seconds: 60.0,
        }]
    );
}

struct RecordingServiceSink(Mutex<Vec<ServiceEvent>>);

impl ServiceEventSink for RecordingServiceSink {
    fn emit(&self, event: &ServiceEvent) -> Result<(), ServiceEventError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.clone());
        Ok(())
    }
}

impl RecordingServiceSink {
    fn events(&self) -> Vec<ServiceEvent> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

struct ServiceQueueSink {
    worker: Arc<LoggingWorker<Delivery>>,
    sink: Arc<dyn ServiceEventSink>,
    registry: Arc<CallbackRegistry>,
}

impl PodLockEventSink for ServiceQueueSink {
    fn emit(&self, event: PodLockEvent) {
        let now = Utc::now().fixed_offset();
        let delivery = Delivery {
            sink: Arc::clone(&self.sink),
            registry: Arc::clone(&self.registry),
            event: ServiceEvent {
                call_id: Uuid::new_v4().to_string(),
                payload: ServiceLoggerPayload {
                    is_error: false,
                    error: None,
                    service: ServiceType::PodLockManager,
                    duration: event.duration_seconds,
                    call_type: event.call_type.as_str().to_owned(),
                    caller: None,
                    event_metadata: Some(ServiceEventMetadata {
                        gauge_labels: Some(event.gauge_labels),
                        gauge_value: Some(event.gauge_value),
                    }),
                },
                start_time: now,
                end_time: now,
            },
        };
        let _admission = self.worker.enqueue(delivery);
    }
}

struct LeaseDelivery {
    worker: Arc<LoggingWorker<Delivery>>,
    recorder: Arc<RecordingServiceSink>,
    sink: Arc<dyn PodLockEventSink>,
}

fn lease_delivery() -> LeaseDelivery {
    let recorder = Arc::new(RecordingServiceSink(Mutex::new(Vec::new())));
    let recorded: Arc<dyn ServiceEventSink> = Arc::<RecordingServiceSink>::clone(&recorder);
    let registry = Arc::new(CallbackRegistry::new(16));
    let worker = Arc::new(
        LoggingWorker::<Delivery>::start(WorkerSettings {
            max_queue_size: 16,
            concurrency: 2,
            event_timeout: DEFAULT_EVENT_TIMEOUT,
            shutdown_drain: DEFAULT_SHUTDOWN_DRAIN,
            timeout_summary_window: DEFAULT_TIMEOUT_SUMMARY_WINDOW,
        })
        .unwrap(),
    );
    let sink: Arc<dyn PodLockEventSink> = Arc::new(ServiceQueueSink {
        worker: Arc::clone(&worker),
        sink: recorded,
        registry,
    });
    LeaseDelivery {
        worker,
        recorder,
        sink,
    }
}

fn assert_lease_event(event: &ServiceEvent, call_type: &str, labels: &str, gauge_value: f64) {
    assert_eq!(event.payload.service, ServiceType::PodLockManager);
    assert_eq!(event.payload.call_type, call_type);
    assert_eq!(event.payload.duration.to_bits(), 60.0_f64.to_bits());
    assert_eq!(
        event.payload.event_metadata,
        Some(ServiceEventMetadata {
            gauge_labels: Some(labels.to_owned()),
            gauge_value: Some(gauge_value),
        })
    );
}

#[tokio::test]
async fn lease_events_reach_the_owned_logging_delivery() {
    let fixture = start_fixture().await;
    let cache = fixture.cache(None).await.unwrap();
    let delivery = lease_delivery();
    let manager = PodLockManager::new(
        Some(Arc::new(RedisLockCache {
            cache: Arc::clone(&cache),
        })),
        60,
        Arc::clone(&delivery.sink),
        PodId::try_from(Uuid::new_v4()).unwrap(),
    );
    assert_eq!(
        manager.acquire_lock("spend:reset", Some(120), true).await,
        Some(true)
    );
    assert_eq!(
        manager.acquire_lock("spend:reset", Some(5), true).await,
        Some(true)
    );
    manager.release_lock("spend:reset").await;
    assert_eq!(delivery.worker.flush().await.completed, 2);
    let events = delivery.recorder.events();
    assert_eq!(events.len(), 2);
    let labels = format!("spend:reset:{}", manager.pod_id);
    assert_lease_event(&events[0], "_emit_acquired_lock_event", &labels, 1.0);
    assert_lease_event(&events[1], "_emit_released_lock_event", &labels, 0.0);
    assert_eq!(delivery.worker.stop().await.completed, 2);
}

#[tokio::test]
async fn shutdown_stops_serving_and_removes_its_directory() {
    let mut fixture = start_fixture().await;
    let address = fixture.address;
    let directory = fixture.directory.as_ref().unwrap().path().to_path_buf();
    let pid = fixture.server.id();
    assert_ne!(pid, std::process::id());
    {
        let cache = fixture.cache(None).await.unwrap();
        assert!(cache.admin.ping(None).await.unwrap());
    }
    fixture.shutdown().unwrap();
    assert!(fixture.server.try_wait().unwrap().is_some());
    #[cfg(target_os = "linux")]
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    assert!(!directory.exists());
    assert!(TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err());
    fixture.shutdown().unwrap();
}

#[test]
fn drop_stops_serving_and_removes_its_directory() {
    let fixture = OracleRedis::start().unwrap();
    let address = fixture.address;
    let directory = fixture.directory.as_ref().unwrap().path().to_path_buf();
    let pid = fixture.server.id();
    assert_ne!(pid, std::process::id());
    drop(fixture);
    #[cfg(target_os = "linux")]
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    assert!(!directory.exists());
    assert!(TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err());
}
