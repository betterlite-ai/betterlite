#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::worker::Rejection;
use secrecy::SecretString;

use super::support::{
    Fixture, MapBuilder, MapResolver, PanickingRepository, RecordingLogger, RecordingRepository,
    capture, disabled, enabled, fixture, logger_entry, record,
};
use crate::audit::callbacks::AuditCallbackEntry;
use crate::audit::service::{AuditService, AuditSubmission};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::identity::CallerIdentity;
use crate::audit_events::sink::AuditLogRepository;
use crate::audit_events::sink::ObjectAuditLog;
use crate::python_hooks::slots::LoadedCallback;

const BEFORE: &str = r#"{"api_key": "sk-1234567890", "name": "team"}"#;
const AFTER: &str = r#"{"api_key": "sk-1234567890"}"#;

fn identity(user_id: Option<&str>, metadata: Option<&str>) -> CallerIdentity {
    CallerIdentity {
        user_id: user_id.map(str::to_owned),
        api_key: Some(SecretString::from("sk-key".to_owned())),
        metadata: metadata.map(|text| serde_json::from_str(text).unwrap()),
        team_metadata: None,
    }
}

fn object<'entry>(
    caller: &'entry CallerIdentity,
    header: Option<&'entry str>,
) -> ObjectAuditLog<'entry> {
    ObjectAuditLog {
        object_id: "team-1",
        action: AuditAction::Updated,
        litellm_changed_by: header,
        caller,
        litellm_proxy_admin_name: Some("proxy-admin"),
        table_name: AuditTable::Team,
        before_value: Some(BEFORE),
        after_value: Some(AFTER),
    }
}

fn repository(
    recorder: &Arc<RecordingLogger>,
    order: Arc<Mutex<Vec<String>>>,
    require_callback_first: bool,
) -> Arc<RecordingRepository> {
    Arc::new(RecordingRepository {
        require_callback_first,
        callback_reached: Arc::clone(&recorder.reached),
        order,
        ..RecordingRepository::new(true, false)
    })
}

fn trait_repository(repository: &Arc<RecordingRepository>) -> Arc<dyn AuditLogRepository> {
    let repository: Arc<dyn AuditLogRepository> = Arc::clone(repository);
    repository
}

fn logger_callback(recorder: &Arc<RecordingLogger>) -> AuditCallbackEntry {
    let logger: Arc<dyn CallbackLogger> = Arc::clone(recorder);
    AuditCallbackEntry::Logger(logger)
}

fn flush(service: &AuditService) -> impl std::future::Future<Output = ()> + '_ {
    async {
        service.writes.flush().await;
        service.delivery.worker.flush().await;
    }
}

#[tokio::test]
async fn object_events_emit_the_pinned_payload_shape() {
    let (entry, recorder) = logger_entry("first", false);
    let repository = Arc::new(RecordingRepository::new(false, false));
    let service = Fixture {
        callbacks: vec![entry],
        repository: trait_repository(&repository),
        ..fixture()
    }
    .start();
    let caller = identity(
        Some("user-1"),
        Some(r#"{"allow_litellm_changed_by_header": true}"#),
    );
    service
        .create_object_audit_log(object(&caller, Some("header-actor")))
        .await;
    flush(&service).await;
    let delivered = recorder.delivered.lock().unwrap();
    assert_eq!(delivered.len(), 1);
    let payload = &delivered[0];
    assert_eq!(payload.id, "audit-id-1");
    assert_eq!(payload.updated_at, "2024-05-06T07:08:09.123000+00:00");
    assert_eq!(payload.changed_by, "header-actor");
    assert_eq!(payload.hashed_changed_by_api_key, "sk-key");
    assert_eq!(payload.action, "updated");
    assert_eq!(payload.table_name, "LiteLLM_TeamTable");
    assert_eq!(payload.object_id, "team-1");
    assert_eq!(
        payload.before_value.as_deref(),
        Some(r#"{"api_key":"sk-1*****7890","name":"team"}"#)
    );
    assert_eq!(
        payload.updated_values.as_deref(),
        Some(r#"{"api_key":"sk-1*****7890"}"#)
    );
}

#[tokio::test]
async fn only_a_boolean_metadata_flag_permits_the_changed_by_header() {
    let (entry, recorder) = logger_entry("first", false);
    let repository = Arc::new(RecordingRepository::new(false, false));
    let service = Fixture {
        callbacks: vec![entry],
        repository: trait_repository(&repository),
        ..fixture()
    }
    .start();
    let truthy = identity(
        Some("user-1"),
        Some(r#"{"allow_litellm_changed_by_header": "true"}"#),
    );
    service
        .create_object_audit_log(object(&truthy, Some("header-actor")))
        .await;
    let empty = identity(None, Some(r#"{"allow_litellm_changed_by_header": true}"#));
    service
        .create_object_audit_log(object(&empty, Some("")))
        .await;
    flush(&service).await;
    let delivered = recorder.delivered.lock().unwrap();
    assert_eq!(delivered.len(), 2);
    assert_eq!(delivered[0].changed_by, "user-1");
    assert_eq!(delivered[1].changed_by, "proxy-admin");
}

#[tokio::test]
async fn a_disabled_policy_keeps_callbacks_and_storage_quiet() {
    let (entry, recorder) = logger_entry("first", false);
    let repository = Arc::new(RecordingRepository::new(true, false));
    let service = Fixture {
        settings: disabled(),
        callbacks: vec![entry],
        repository: trait_repository(&repository),
        ..fixture()
    }
    .start();
    let caller = identity(Some("user-1"), None);
    service.create_object_audit_log(object(&caller, None)).await;
    assert_eq!(
        service.try_submit(record()),
        AuditSubmission::Disabled,
        "a disabled policy rejects every submission"
    );
    flush(&service).await;
    assert!(recorder.delivered.lock().unwrap().is_empty());
    assert!(repository.inserted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_object_value_that_is_not_json_is_logged_and_skipped() {
    let (captured, guard) = capture();
    let (entry, recorder) = logger_entry("first", false);
    let repository = Arc::new(RecordingRepository::new(true, false));
    let service = Fixture {
        callbacks: vec![entry],
        repository: trait_repository(&repository),
        ..fixture()
    }
    .start();
    let caller = identity(Some("user-1"), None);
    let mut broken = object(&caller, None);
    broken.before_value = Some("{not json");
    service.create_object_audit_log(broken).await;
    flush(&service).await;
    drop(guard);
    assert!(recorder.delivered.lock().unwrap().is_empty());
    assert!(repository.inserted.lock().unwrap().is_empty());
    assert!(captured.text().contains("Failed Creating audit log"));
}

#[tokio::test]
async fn the_debug_line_withholds_the_actor_key() {
    let (captured, guard) = capture();
    let repository = Arc::new(RecordingRepository::new(true, false));
    let service = Fixture {
        repository: trait_repository(&repository),
        ..fixture()
    }
    .start();
    let mut stored = record();
    stored.changed_by_api_key = Some(SecretString::from("sk-secret-value".to_owned()));
    service.create_audit_log_for_update(stored).await;
    flush(&service).await;
    drop(guard);
    let output = captured.text();
    assert!(output.contains("creating audit log"), "{output}");
    assert!(!output.contains("sk-secret-value"), "{output}");
}

#[tokio::test]
async fn the_callback_runs_while_the_database_write_is_pending() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::new(RecordingLogger::watching("first", Arc::clone(&order)));
    let repository = repository(&recorder, Arc::clone(&order), true);
    let service = Fixture {
        callbacks: vec![logger_callback(&recorder)],
        repository: trait_repository(&repository),
        ..fixture()
    }
    .start();
    service.create_audit_log_for_update(record()).await;
    flush(&service).await;
    assert_eq!(
        *order.lock().unwrap(),
        vec!["callback:first".to_owned(), "insert:true".to_owned()]
    );
    assert_eq!(repository.inserted.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_missing_database_is_logged_after_delivery() {
    let (captured, guard) = capture();
    let (entry, recorder) = logger_entry("first", false);
    let repository = Arc::new(RecordingRepository::new(false, false));
    let service = Fixture {
        callbacks: vec![entry],
        repository: trait_repository(&repository),
        ..fixture()
    }
    .start();
    service.create_audit_log_for_update(record()).await;
    flush(&service).await;
    drop(guard);
    assert_eq!(recorder.delivered.lock().unwrap().len(), 1);
    assert!(
        captured
            .text()
            .contains("prisma_client is None, cannot write audit log to DB")
    );
}

#[tokio::test]
async fn a_failed_insert_does_not_cancel_delivery() {
    let (captured, guard) = capture();
    let (entry, recorder) = logger_entry("first", false);
    let repository = Arc::new(RecordingRepository::new(true, true));
    let service = Fixture {
        callbacks: vec![entry],
        repository: trait_repository(&repository),
        ..fixture()
    }
    .start();
    service.create_audit_log_for_update(record()).await;
    flush(&service).await;
    drop(guard);
    assert_eq!(recorder.delivered.lock().unwrap().len(), 1);
    assert!(captured.text().contains("Failed Creating audit log"));
}

#[tokio::test]
async fn unknown_named_callbacks_are_skipped_and_the_database_still_writes() {
    let (captured, guard) = capture();
    let repository = Arc::new(RecordingRepository::new(true, false));
    let service = Fixture {
        callbacks: vec![AuditCallbackEntry::Named("missing".to_owned())],
        repository: trait_repository(&repository),
        ..fixture()
    }
    .start();
    service.create_audit_log_for_update(record()).await;
    flush(&service).await;
    drop(guard);
    assert_eq!(repository.inserted.lock().unwrap().len(), 1);
    assert!(
        captured
            .text()
            .contains("Could not resolve audit log callback: missing")
    );
}

#[tokio::test]
async fn the_configured_s3_parameters_build_the_dedicated_logger() {
    let dedicated = Arc::new(RecordingLogger::new("dedicated", false));
    let logger: Arc<dyn CallbackLogger> = Arc::clone(&dedicated);
    let builder = Arc::new(MapBuilder {
        logger: Some(logger),
        calls: Mutex::new(Vec::new()),
    });
    let resolver = Arc::new(MapResolver::default());
    let resolver_dyn: Arc<dyn LoggerResolver> = Arc::clone(&resolver);
    let params: betterlite_config::sections::settings::JsonValue =
        serde_json::from_str(r#"{"s3_bucket_name": "audit-bucket"}"#).unwrap();
    let mut settings = enabled();
    settings.config.s3_audit_callback_params = Some(params.clone());
    let service = Fixture {
        settings,
        callbacks: vec![AuditCallbackEntry::Named("s3_v2".to_owned())],
        resolver: resolver_dyn,
        dedicated: builder.clone(),
        ..fixture()
    }
    .start();
    service.create_audit_log_for_update(record()).await;
    flush(&service).await;
    assert_eq!(*builder.calls.lock().unwrap(), vec![params]);
    assert!(resolver.calls.lock().unwrap().is_empty());
    assert_eq!(dedicated.delivered.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn without_s3_audit_parameters_the_shared_resolver_answers() {
    let shared = Arc::new(RecordingLogger::new("s3_v2", false));
    let logger: Arc<dyn CallbackLogger> = Arc::clone(&shared);
    let resolver = Arc::new(MapResolver::with(vec![("s3_v2", logger)]));
    let resolver_dyn: Arc<dyn LoggerResolver> = Arc::clone(&resolver);
    let builder = Arc::new(MapBuilder::default());
    let service = Fixture {
        callbacks: vec![AuditCallbackEntry::Named("s3_v2".to_owned())],
        resolver: resolver_dyn,
        dedicated: builder.clone(),
        ..fixture()
    }
    .start();
    service.create_audit_log_for_update(record()).await;
    flush(&service).await;
    assert!(builder.calls.lock().unwrap().is_empty());
    assert_eq!(*resolver.calls.lock().unwrap(), vec!["s3_v2".to_owned()]);
    assert_eq!(shared.delivered.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn clearing_and_appending_callbacks_follow_the_configured_list() {
    let (entry, initial) = logger_entry("initial", false);
    let otel = Arc::new(RecordingLogger::new("otel", false));
    let otel_logger: Arc<dyn CallbackLogger> = Arc::clone(&otel);
    let resolver = Arc::new(MapResolver::with(vec![("otel", otel_logger)]));
    let resolver_dyn: Arc<dyn LoggerResolver> = Arc::clone(&resolver);
    let repository = Arc::new(RecordingRepository::new(true, false));
    let service = Fixture {
        callbacks: vec![entry],
        resolver: resolver_dyn,
        repository: trait_repository(&repository),
        ..fixture()
    }
    .start();
    service.create_audit_log_for_update(record()).await;
    service.clear_audit_log_callbacks();
    service.create_audit_log_for_update(record()).await;
    service.append_audit_log_callback(LoadedCallback::Named("otel".to_owned()));
    service.create_audit_log_for_update(record()).await;
    flush(&service).await;
    assert_eq!(initial.delivered.lock().unwrap().len(), 1);
    assert_eq!(otel.delivered.lock().unwrap().len(), 1);
    assert_eq!(*resolver.calls.lock().unwrap(), vec!["otel".to_owned()]);
    service.delivery.named.reset_audit_log_callback_cache();
    service.create_audit_log_for_update(record()).await;
    flush(&service).await;
    assert_eq!(
        *resolver.calls.lock().unwrap(),
        vec!["otel".to_owned(), "otel".to_owned()]
    );
}

#[tokio::test]
async fn try_submit_accepts_the_record_and_writes_it_in_the_background() {
    let (entry, recorder) = logger_entry("first", false);
    let repository = Arc::new(RecordingRepository::new(true, false));
    let service = Fixture {
        callbacks: vec![entry],
        repository: trait_repository(&repository),
        ..fixture()
    }
    .start();
    assert_eq!(service.try_submit(record()), AuditSubmission::Accepted);
    flush(&service).await;
    let inserted = repository.inserted.lock().unwrap();
    assert_eq!(inserted.len(), 1);
    assert_eq!(inserted[0].id, "audit-1");
    assert_eq!(recorder.delivered.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn try_submit_reports_a_full_queue_and_the_record_reaches_nothing() {
    let (entry, recorder) = logger_entry("first", false);
    let repository = Arc::new(RecordingRepository::new(true, false));
    let service = Fixture {
        callbacks: vec![entry],
        repository: trait_repository(&repository),
        queue: 1,
        ..fixture()
    }
    .start();
    assert_eq!(service.try_submit(record()), AuditSubmission::Accepted);
    let mut second = record();
    second.id = "audit-2".to_owned();
    assert_eq!(
        service.try_submit(second),
        AuditSubmission::Rejected(Rejection::QueueFull)
    );
    flush(&service).await;
    let inserted = repository.inserted.lock().unwrap();
    assert_eq!(inserted.len(), 1);
    assert_eq!(inserted[0].id, "audit-1");
    assert_eq!(recorder.delivered.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_rejected_callback_delivery_still_inserts_the_record() {
    let (captured, guard) = capture();
    let (first, first_recorder) = logger_entry("first", false);
    let (second, second_recorder) = logger_entry("second", false);
    let repository = Arc::new(RecordingRepository::new(true, false));
    let service = Fixture {
        callbacks: vec![first, second],
        repository: trait_repository(&repository),
        queue: 1,
        ..fixture()
    }
    .start();
    service.create_audit_log_for_update(record()).await;
    flush(&service).await;
    drop(guard);
    assert_eq!(repository.inserted.lock().unwrap().len(), 1);
    assert_eq!(first_recorder.delivered.lock().unwrap().len(), 1);
    assert!(second_recorder.delivered.lock().unwrap().is_empty());
    assert!(
        captured
            .text()
            .contains("the audit log callback was rejected")
    );
}

#[tokio::test]
async fn shutdown_drains_accepted_writes_and_callback_deliveries() {
    let (entry, recorder) = logger_entry("first", false);
    let repository = Arc::new(RecordingRepository::new(true, false));
    let service = Fixture {
        callbacks: vec![entry],
        repository: trait_repository(&repository),
        ..fixture()
    }
    .start();
    assert_eq!(service.try_submit(record()), AuditSubmission::Accepted);
    let report = service.shutdown().await;
    assert_eq!(report.completed, 1);
    assert_eq!(repository.inserted.lock().unwrap().len(), 1);
    assert_eq!(recorder.delivered.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_panicking_write_is_reported_under_the_audit_write_job_name() {
    let (captured, guard) = capture();
    let service = Fixture {
        repository: Arc::new(PanickingRepository),
        ..fixture()
    }
    .start();
    assert_eq!(service.try_submit(record()), AuditSubmission::Accepted);
    service.writes.flush().await;
    drop(guard);
    let output = captured.text();
    assert!(
        output.contains("a logging callback task failed"),
        "{output}"
    );
    assert!(output.contains("callback=audit_log_write"), "{output}");
}
