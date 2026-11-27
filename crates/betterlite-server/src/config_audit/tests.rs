#![cfg(test)]

mod support;

use std::sync::{Arc, Mutex, PoisonError};

use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::record::AuditLogRecord;
use crate::config_audit::{ConfigChange, config_audit_log};

use self::support::{RecordingRepository, caller, enabled_sink, parsed};

fn recording_repository() -> Arc<RecordingRepository> {
    Arc::new(RecordingRepository {
        inserted: Mutex::new(Vec::new()),
    })
}

fn stored_rows(repository: &RecordingRepository) -> Vec<AuditLogRecord> {
    repository
        .inserted
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

#[tokio::test]
async fn config_audit_log_stores_the_config_row_with_redacted_values() {
    let repository = recording_repository();
    let sink = enabled_sink(Arc::clone(&repository));
    let identity = caller(None);
    let before = parsed(r#"{"database_url":"postgres://user:pass@host/db","model":"gpt-4o"}"#);
    let after = parsed(r#"{"database_url":"postgres://user:pass@host/db2","model":"gpt-4o"}"#);
    config_audit_log(
        &sink,
        ConfigChange {
            param: "general_settings",
            action: AuditAction::Updated,
            table: AuditTable::Config,
            before: Some(&before),
            after: Some(&after),
            caller: &identity,
        },
    )
    .await;
    let rows = stored_rows(&repository);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].object_id, "general_settings");
    assert_eq!(rows[0].action, AuditAction::Updated);
    assert_eq!(rows[0].table_name, AuditTable::Config);
    assert_eq!(
        rows[0].before_value,
        Some(parsed(r#"{"database_url":"REDACTED","model":"gpt-4o"}"#))
    );
    assert_eq!(
        rows[0].updated_values,
        Some(parsed(r#"{"database_url":"REDACTED","model":"gpt-4o"}"#))
    );
}

#[tokio::test]
async fn config_audit_log_hides_every_environment_variable_value_on_both_sides() {
    let repository = recording_repository();
    let sink = enabled_sink(Arc::clone(&repository));
    let identity = caller(None);
    let before = parsed(r#"{"DATABASE_URL":"postgres://user:pass@host/db","PLAIN":"one"}"#);
    let after = parsed(r#"{"DATABASE_URL":"postgres://user:pass@host/db2","PLAIN":"two"}"#);
    config_audit_log(
        &sink,
        ConfigChange {
            param: "environment_variables",
            action: AuditAction::Updated,
            table: AuditTable::Config,
            before: Some(&before),
            after: Some(&after),
            caller: &identity,
        },
    )
    .await;
    let rows = stored_rows(&repository);
    assert_eq!(
        rows[0].before_value,
        Some(parsed(r#"{"DATABASE_URL":"REDACTED","PLAIN":"REDACTED"}"#))
    );
    assert_eq!(
        rows[0].updated_values,
        Some(parsed(r#"{"DATABASE_URL":"REDACTED","PLAIN":"REDACTED"}"#))
    );
}

#[tokio::test]
async fn config_audit_log_keeps_an_absent_value_absent() {
    let repository = recording_repository();
    let sink = enabled_sink(Arc::clone(&repository));
    let identity = caller(None);
    let after = parsed(r#"{"database_url":"postgres://user:pass@host/db"}"#);
    config_audit_log(
        &sink,
        ConfigChange {
            param: "general_settings",
            action: AuditAction::Created,
            table: AuditTable::Key,
            before: None,
            after: Some(&after),
            caller: &identity,
        },
    )
    .await;
    let rows = stored_rows(&repository);
    assert_eq!(rows[0].before_value, None);
    assert_eq!(
        rows[0].updated_values,
        Some(parsed(r#"{"database_url":"REDACTED"}"#))
    );
    assert_eq!(rows[0].table_name, AuditTable::Key);
    assert_eq!(rows[0].action, AuditAction::Created);
}

#[tokio::test]
async fn config_audit_log_attributes_a_caller_without_user_id_to_the_proxy_admin() {
    let repository = recording_repository();
    let sink = enabled_sink(Arc::clone(&repository));
    let identity = caller(None);
    config_audit_log(
        &sink,
        ConfigChange {
            param: "litellm_settings",
            action: AuditAction::Updated,
            table: AuditTable::Config,
            before: None,
            after: None,
            caller: &identity,
        },
    )
    .await;
    let rows = stored_rows(&repository);
    assert_eq!(rows[0].changed_by.as_deref(), Some("default_user_id"));
    assert_eq!(rows[0].before_value, None);
    assert_eq!(rows[0].updated_values, None);
}
