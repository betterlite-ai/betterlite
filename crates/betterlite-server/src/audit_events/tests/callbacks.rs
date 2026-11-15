#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio_util::task::TaskTracker;

use super::support::{
    CollectingSpawner, DroppingSpawner, RecordingCallback, SAMPLE_TIMESTAMP, TestRegistry, capture,
    instance, parsed, plain_record, recorder, registry_handle, sample_record, spawner_handle,
};
use crate::audit_events::callbacks::{
    AuditCallbackConfig, AuditCallbackDispatcher, AuditCallbackFuture, AuditLogCallback,
    AuditTaskSpawner as _, TrackerSpawner,
};
use crate::audit_events::payload::build_audit_log_payload;

fn registry() -> Arc<TestRegistry> {
    let logger = recorder("s3", false);
    let registry = TestRegistry {
        instances: Mutex::new(BTreeMap::new()),
        resolves: Mutex::new(Vec::new()),
        s3_calls: Mutex::new(Vec::new()),
        s3_logger: logger,
        s3_fails: false,
    };
    Arc::new(registry)
}

fn register(registry: &TestRegistry, name: &str, callback: Arc<RecordingCallback>) {
    let erased: Arc<dyn AuditLogCallback> = callback;
    registry
        .instances
        .lock()
        .unwrap()
        .insert(name.to_owned(), erased);
}

#[tokio::test]
async fn audit_tracker_spawner_runs_the_task() {
    let tracker = TaskTracker::new();
    let spawner = TrackerSpawner {
        tasks: tracker.clone(),
    };
    let ran = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&ran);
    let task: AuditCallbackFuture = Box::pin(async move {
        flag.store(true, Ordering::SeqCst);
    });
    spawner.spawn(task);
    tracker.close();
    tracker.wait().await;
    assert!(ran.load(Ordering::SeqCst));
}

#[tokio::test]
async fn audit_callback_cache_hit() {
    let registry = registry();
    let otel = recorder("otel", false);
    register(&registry, "otel", Arc::clone(&otel));
    let spawner = Arc::new(CollectingSpawner::default());
    let dispatcher =
        AuditCallbackDispatcher::new(registry_handle(&registry), spawner_handle(&spawner));
    let configs = [AuditCallbackConfig::Named("otel".to_owned())];
    let payload = build_audit_log_payload(&plain_record());
    dispatcher.dispatch(&configs, None, &payload);
    spawner.run_all().await;
    dispatcher.dispatch(&configs, None, &payload);
    spawner.run_all().await;
    assert_eq!(otel.delivered.lock().unwrap().len(), 2);
    assert_eq!(registry.resolves.lock().unwrap().len(), 1);
}

#[test]
fn audit_unresolved_not_cached() {
    let (captured, _guard) = capture();
    let registry = registry();
    let spawner = Arc::new(CollectingSpawner::default());
    let dispatcher =
        AuditCallbackDispatcher::new(registry_handle(&registry), spawner_handle(&spawner));
    let configs = [AuditCallbackConfig::Named("missing".to_owned())];
    let payload = build_audit_log_payload(&plain_record());
    dispatcher.dispatch(&configs, None, &payload);
    dispatcher.dispatch(&configs, None, &payload);
    assert_eq!(registry.resolves.lock().unwrap().len(), 2);
    assert!(spawner.tasks.lock().unwrap().is_empty());
    let text = captured.text();
    assert_eq!(
        text.matches("Could not resolve audit log callback: missing")
            .count(),
        2
    );
}

#[tokio::test]
async fn audit_unresolved_warning() {
    let (captured, guard) = capture();
    let registry = registry();
    let good = recorder("good", false);
    let spawner = Arc::new(CollectingSpawner::default());
    let dispatcher = AuditCallbackDispatcher::new(registry, spawner_handle(&spawner));
    dispatcher.dispatch(
        &[
            AuditCallbackConfig::Named("missing".to_owned()),
            instance(Arc::clone(&good)),
        ],
        None,
        &build_audit_log_payload(&plain_record()),
    );
    spawner.run_all().await;
    assert_eq!(good.delivered.lock().unwrap().len(), 1);
    assert!(spawner.tasks.lock().unwrap().is_empty());
    drop(guard);
    assert!(
        captured
            .text()
            .contains("Could not resolve audit log callback: missing")
    );
}

#[tokio::test]
async fn audit_s3_override_empty() {
    let registry = registry();
    let spawner = Arc::new(CollectingSpawner::default());
    let dispatcher =
        AuditCallbackDispatcher::new(registry_handle(&registry), spawner_handle(&spawner));
    let configs = [AuditCallbackConfig::Named("s3_v2".to_owned())];
    let payload = build_audit_log_payload(&plain_record());
    dispatcher.dispatch(&configs, Some(&parsed("{}")), &payload);
    spawner.run_all().await;
    assert!(registry.resolves.lock().unwrap().is_empty());
    assert_eq!(*registry.s3_calls.lock().unwrap(), vec![parsed("{}")]);
    assert_eq!(registry.s3_logger.delivered.lock().unwrap().len(), 1);
    let factory =
        AuditCallbackDispatcher::new(registry_handle(&registry), spawner_handle(&spawner));
    factory.dispatch(&configs, None, &payload);
    assert_eq!(*registry.resolves.lock().unwrap(), ["s3_v2".to_owned()]);
}

#[tokio::test]
async fn audit_cache_reset() {
    let registry = registry();
    let first = recorder("first", false);
    register(&registry, "otel", Arc::clone(&first));
    let spawner = Arc::new(CollectingSpawner::default());
    let dispatcher =
        AuditCallbackDispatcher::new(registry_handle(&registry), spawner_handle(&spawner));
    let configs = [AuditCallbackConfig::Named("otel".to_owned())];
    let payload = build_audit_log_payload(&plain_record());
    dispatcher.dispatch(&configs, None, &payload);
    spawner.run_all().await;
    let second = recorder("second", false);
    register(&registry, "otel", Arc::clone(&second));
    dispatcher.dispatch(&configs, None, &payload);
    spawner.run_all().await;
    assert_eq!(first.delivered.lock().unwrap().len(), 2);
    assert!(second.delivered.lock().unwrap().is_empty());
    assert_eq!(registry.resolves.lock().unwrap().len(), 1);
    dispatcher.reset_audit_log_callback_cache();
    dispatcher.dispatch(&configs, None, &payload);
    spawner.run_all().await;
    assert_eq!(first.delivered.lock().unwrap().len(), 2);
    assert_eq!(second.delivered.lock().unwrap().len(), 1);
    assert_eq!(registry.resolves.lock().unwrap().len(), 2);
}

#[test]
fn audit_payload_nulls() {
    let payload = build_audit_log_payload(&plain_record());
    assert_eq!(payload.updated_at, SAMPLE_TIMESTAMP);
    assert_eq!(payload.changed_by, "");
    assert_eq!(payload.hashed_changed_by_api_key, "");
    assert_eq!(payload.action, "updated");
    assert_eq!(payload.table_name, "LiteLLM_TeamTable");
    assert_eq!(payload.object_id, "team-1");
    assert_eq!(payload.before_value, None);
    assert_eq!(payload.updated_values, None);
    let masked = build_audit_log_payload(&sample_record(
        Some(r#"{"api_key": "sk-1234567890"}"#),
        Some(r#"{"name": "team"}"#),
    ));
    assert_eq!(
        masked.before_value.as_deref(),
        Some(r#"{"api_key":"sk-1*****7890"}"#)
    );
    assert_eq!(masked.updated_values.as_deref(), Some(r#"{"name":"team"}"#));
    assert_eq!(masked.changed_by, "admin");
    assert_eq!(masked.hashed_changed_by_api_key, "sk-key");
}

#[tokio::test]
async fn audit_callback_order() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::new(RecordingCallback {
        label: "first".to_owned(),
        fails: false,
        delivered: Mutex::new(Vec::new()),
        order: Arc::clone(&order),
    });
    let otel = Arc::new(RecordingCallback {
        label: "otel".to_owned(),
        fails: false,
        delivered: Mutex::new(Vec::new()),
        order: Arc::clone(&order),
    });
    let last = Arc::new(RecordingCallback {
        label: "last".to_owned(),
        fails: false,
        delivered: Mutex::new(Vec::new()),
        order: Arc::clone(&order),
    });
    let registry = registry();
    register(&registry, "otel", otel);
    let spawner = Arc::new(CollectingSpawner::default());
    let dispatcher = AuditCallbackDispatcher::new(registry, spawner_handle(&spawner));
    let configs = [
        instance(Arc::clone(&first)),
        AuditCallbackConfig::Named("otel".to_owned()),
        instance(Arc::clone(&last)),
    ];
    dispatcher.dispatch(&configs, None, &build_audit_log_payload(&plain_record()));
    spawner.run_all().await;
    assert_eq!(*order.lock().unwrap(), ["first", "otel", "last"]);
    assert_eq!(first.delivered.lock().unwrap().len(), 1);
    assert_eq!(last.delivered.lock().unwrap().len(), 1);
}

#[test]
fn audit_invalid_callback_ignored() {
    let (captured, _guard) = capture();
    let registry = registry();
    let spawner = Arc::new(CollectingSpawner::default());
    let dispatcher =
        AuditCallbackDispatcher::new(registry_handle(&registry), spawner_handle(&spawner));
    dispatcher.dispatch(
        &[AuditCallbackConfig::Other],
        None,
        &build_audit_log_payload(&plain_record()),
    );
    assert!(registry.resolves.lock().unwrap().is_empty());
    assert!(spawner.tasks.lock().unwrap().is_empty());
    let text = captured.text();
    assert!(!text.contains("Could not resolve audit log callback"));
    assert!(!text.contains("Failed dispatching audit log to callback"));
}

#[tokio::test]
async fn audit_dispatch_exception_continues() {
    let (captured, _guard) = capture();
    let registry = Arc::new(TestRegistry {
        instances: Mutex::new(BTreeMap::new()),
        resolves: Mutex::new(Vec::new()),
        s3_calls: Mutex::new(Vec::new()),
        s3_logger: recorder("s3", false),
        s3_fails: true,
    });
    let otel = recorder("otel", false);
    register(&registry, "otel", Arc::clone(&otel));
    let spawner = Arc::new(CollectingSpawner::default());
    let dispatcher =
        AuditCallbackDispatcher::new(registry_handle(&registry), spawner_handle(&spawner));
    let configs = [
        AuditCallbackConfig::Named("s3_v2".to_owned()),
        AuditCallbackConfig::Named("otel".to_owned()),
    ];
    dispatcher.dispatch(
        &configs,
        Some(&parsed(r#"{"bucket_name": "audit"}"#)),
        &build_audit_log_payload(&plain_record()),
    );
    spawner.run_all().await;
    assert_eq!(otel.delivered.lock().unwrap().len(), 1);
    assert!(
        captured
            .text()
            .contains("Failed dispatching audit log to callback: the audit log callback failed")
    );
}

#[tokio::test]
async fn audit_task_cancel_or_fail() {
    let (cancelled, cancelled_guard) = capture();
    let registry = registry();
    let dispatcher =
        AuditCallbackDispatcher::new(registry_handle(&registry), Arc::new(DroppingSpawner));
    let configs = [AuditCallbackConfig::Named("missing".to_owned())];
    let payload = build_audit_log_payload(&plain_record());
    let good = recorder("good", false);
    register(&registry, "good", Arc::clone(&good));
    dispatcher.dispatch(&[instance(Arc::clone(&good))], None, &payload);
    dispatcher.dispatch(&configs, None, &payload);
    assert!(!cancelled.text().contains("Audit log callback task failed"));
    assert!(good.delivered.lock().unwrap().is_empty());
    drop(cancelled_guard);

    let (failed, failed_guard) = capture();
    let spawner = Arc::new(CollectingSpawner::default());
    let failing = recorder("failing", true);
    let succeeding = recorder("succeeding", false);
    let retry = AuditCallbackDispatcher::new(registry, spawner_handle(&spawner));
    retry.dispatch(
        &[instance(failing), instance(Arc::clone(&succeeding))],
        None,
        &payload,
    );
    spawner.run_all().await;
    assert_eq!(succeeding.delivered.lock().unwrap().len(), 1);
    assert!(
        failed
            .text()
            .contains("Audit log callback task failed: the audit log callback failed")
    );
    drop(failed_guard);
}
