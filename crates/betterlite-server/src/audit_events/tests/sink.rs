#![cfg(test)]

use std::sync::{Arc, Mutex};

use chrono::DateTime;
use secrecy::{ExposeSecret, SecretString};

use super::support::{
    CollectingSpawner, RecordingCallback, RecordingRepository, SAMPLE_TIMESTAMP, TestSettings,
    audit_sink, capture, instance, parsed, plain_record, recorder, recording_repository,
    sample_record,
};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::identity::CallerIdentity;
use crate::audit_events::sink::{AuditEventSink, ObjectAuditLog};

async fn log_team_update(sink: &AuditEventSink, caller: &CallerIdentity, changed_by: Option<&str>) {
    sink.create_object_audit_log(ObjectAuditLog {
        object_id: "team-1",
        action: AuditAction::Updated,
        litellm_changed_by: changed_by,
        caller,
        litellm_proxy_admin_name: Some("admin"),
        table_name: AuditTable::Team,
        before_value: Some(r#"{"api_key": "sk-1234567890"}"#),
        after_value: Some(r#"{"name": "team"}"#),
    })
    .await;
}

fn caller() -> CallerIdentity {
    CallerIdentity {
        user_id: Some("user-1".to_owned()),
        api_key: Some(SecretString::from("sk-key".to_owned())),
        metadata: Some(parsed(r#"{"allow_litellm_changed_by_header": true}"#)),
        team_metadata: None,
    }
}

fn enabled_sink(
    callback: &Arc<RecordingCallback>,
    repository: &Arc<RecordingRepository>,
    spawner: &Arc<CollectingSpawner>,
) -> AuditEventSink {
    audit_sink(
        TestSettings {
            enabled: true,
            callbacks: vec![instance(Arc::clone(callback))],
            s3: None,
        },
        Arc::clone(repository),
        Arc::clone(callback),
        spawner,
    )
}

fn assert_stored(repository: &RecordingRepository) {
    let inserted = repository.inserted.lock().unwrap().clone();
    assert_eq!(inserted.len(), 1);
    assert_eq!(
        inserted[0].updated_at,
        DateTime::parse_from_rfc3339(SAMPLE_TIMESTAMP).unwrap()
    );
    assert_eq!(inserted[0].changed_by.as_deref(), Some("ops"));
    assert_eq!(
        inserted[0]
            .changed_by_api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-key")
    );
    assert_eq!(inserted[0].action, AuditAction::Updated);
    assert_eq!(inserted[0].table_name, AuditTable::Team);
    assert_eq!(inserted[0].object_id, "team-1");
    assert_eq!(
        inserted[0].before_value,
        Some(parsed(r#"{"api_key": "sk-1*****7890"}"#))
    );
    assert_eq!(
        inserted[0].updated_values,
        Some(parsed(r#"{"name": "team"}"#))
    );
}

#[tokio::test]
async fn audit_object_projection() {
    let callback = recorder("otel", false);
    let repository = recording_repository();
    let spawner = Arc::new(CollectingSpawner::default());
    let sink = enabled_sink(&callback, &repository, &spawner);
    log_team_update(&sink, &caller(), Some("ops")).await;
    spawner.run_all().await;
    assert_stored(&repository);
    let delivered = callback.delivered.lock().unwrap().clone();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].changed_by, "ops");
    assert_eq!(
        delivered[0].before_value.as_deref(),
        Some(r#"{"api_key":"sk-1*****7890"}"#)
    );
    assert_eq!(delivered[0].updated_at, SAMPLE_TIMESTAMP);
}

#[tokio::test]
async fn audit_debug_log_redacts_the_api_key() {
    let (captured, guard) = capture();
    let callback = recorder("otel", false);
    let repository = recording_repository();
    let spawner = Arc::new(CollectingSpawner::default());
    let sink = enabled_sink(&callback, &repository, &spawner);
    sink.create_audit_log_for_update(sample_record(None, None))
        .await;
    spawner.run_all().await;
    drop(guard);
    let text = captured.text();
    assert!(text.contains("creating audit log for"));
    assert!(text.contains("[REDACTED]"));
    assert!(!text.contains("sk-key"));
}

#[tokio::test]
async fn audit_invalid_json_is_logged() {
    let (captured, guard) = capture();
    let callback = recorder("otel", false);
    let repository = recording_repository();
    let spawner = Arc::new(CollectingSpawner::default());
    let sink = enabled_sink(&callback, &repository, &spawner);
    sink.create_object_audit_log(ObjectAuditLog {
        object_id: "team-1",
        action: AuditAction::Updated,
        litellm_changed_by: None,
        caller: &caller(),
        litellm_proxy_admin_name: Some("admin"),
        table_name: AuditTable::Team,
        before_value: Some("{oops"),
        after_value: None,
    })
    .await;
    spawner.run_all().await;
    assert!(repository.inserted.lock().unwrap().is_empty());
    assert!(callback.delivered.lock().unwrap().is_empty());
    drop(guard);
    assert!(captured.text().contains("Failed Creating audit log"));
}

#[tokio::test]
async fn audit_disabled_no_effects() {
    let callback = recorder("otel", false);
    let repository = recording_repository();
    let spawner = Arc::new(CollectingSpawner::default());
    let sink = audit_sink(
        TestSettings {
            enabled: false,
            callbacks: vec![instance(Arc::clone(&callback))],
            s3: None,
        },
        Arc::clone(&repository),
        Arc::clone(&callback),
        &spawner,
    );
    sink.create_audit_log_for_update(plain_record()).await;
    log_team_update(&sink, &caller(), None).await;
    assert!(spawner.tasks.lock().unwrap().is_empty());
    assert!(repository.inserted.lock().unwrap().is_empty());
    assert!(callback.delivered.lock().unwrap().is_empty());
}

#[tokio::test]
async fn audit_callback_before_database() {
    let callback = recorder("otel", false);
    let repository = Arc::new(RecordingRepository {
        available: false,
        fail: false,
        inserted: Mutex::new(Vec::new()),
    });
    let spawner = Arc::new(CollectingSpawner::default());
    let sink = enabled_sink(&callback, &repository, &spawner);
    sink.create_audit_log_for_update(plain_record()).await;
    spawner.run_all().await;
    assert_eq!(callback.delivered.lock().unwrap().len(), 1);
    assert!(repository.inserted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn audit_no_database_logs() {
    let (captured, guard) = capture();
    let callback = recorder("otel", false);
    let repository = Arc::new(RecordingRepository {
        available: false,
        fail: false,
        inserted: Mutex::new(Vec::new()),
    });
    let spawner = Arc::new(CollectingSpawner::default());
    let sink = enabled_sink(&callback, &repository, &spawner);
    sink.create_audit_log_for_update(plain_record()).await;
    assert!(repository.inserted.lock().unwrap().is_empty());
    drop(guard);
    assert!(
        captured
            .text()
            .contains("prisma_client is None, cannot write audit log to DB")
    );
}

#[tokio::test]
async fn audit_insert_failure_is_nonblocking() {
    let (captured, guard) = capture();
    let callback = recorder("otel", false);
    let repository = Arc::new(RecordingRepository {
        available: true,
        fail: true,
        inserted: Mutex::new(Vec::new()),
    });
    let spawner = Arc::new(CollectingSpawner::default());
    let sink = enabled_sink(&callback, &repository, &spawner);
    sink.create_audit_log_for_update(plain_record()).await;
    spawner.run_all().await;
    assert!(repository.inserted.lock().unwrap().is_empty());
    assert_eq!(callback.delivered.lock().unwrap().len(), 1);
    drop(guard);
    assert!(captured.text().contains("Failed Creating audit log"));
}
