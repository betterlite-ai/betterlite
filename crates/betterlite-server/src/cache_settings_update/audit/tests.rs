#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;
use secrecy::{ExposeSecret as _, SecretString};

use super::{
    CacheSettingsAudit, CacheSettingsAuditEntry, SinkCacheSettingsAudit, spawn_cache_settings_audit,
};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::cache_settings_update::support;

fn entry(
    action: AuditAction,
    header: Option<&str>,
    user_id: Option<&str>,
) -> CacheSettingsAuditEntry {
    CacheSettingsAuditEntry {
        action,
        litellm_changed_by: header.map(str::to_owned),
        user_id: user_id.map(str::to_owned),
        api_key: Some(SecretString::from("hashed-token")),
        before_value: String::from(r#"{"settings":{}}"#),
        updated_values: String::from(r#"{"settings":{"host":"***REDACTED***"}}"#),
    }
}

async fn changed_by_for(header: Option<&str>, user_id: Option<&str>) -> Option<String> {
    let (sink, repository) = support::sink(true);
    let audit = SinkCacheSettingsAudit { sink };
    audit
        .write(entry(AuditAction::Created, header, user_id))
        .await
        .expect("the record is written");
    repository.records()[0].changed_by.clone()
}

#[tokio::test]
async fn record_carries_the_cache_config_row_fields() {
    let (sink, repository) = support::sink(true);
    let audit = SinkCacheSettingsAudit { sink };
    audit
        .write(entry(
            AuditAction::Updated,
            Some("reviewer"),
            Some("admin-user"),
        ))
        .await
        .expect("the record is written");
    let records = repository.records();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.id, "audit-id");
    assert_eq!(record.updated_at.to_rfc3339(), "2024-05-01T00:00:00+00:00");
    assert_eq!(record.changed_by.as_deref(), Some("reviewer"));
    assert_eq!(
        record
            .changed_by_api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("hashed-token".to_owned())
    );
    assert_eq!(record.action, AuditAction::Updated);
    assert_eq!(record.table_name, AuditTable::CacheConfig);
    assert_eq!(record.table_name.as_str(), "LiteLLM_CacheConfig");
    assert_eq!(record.object_id, "cache_config");
    assert_eq!(
        record.before_value,
        Some(JsonValue::String(r#"{"settings":{}}"#.to_owned()))
    );
    assert_eq!(
        record.updated_values,
        Some(JsonValue::String(
            r#"{"settings":{"host":"***REDACTED***"}}"#.to_owned()
        ))
    );
}

#[tokio::test]
async fn changed_by_prefers_the_header_then_the_user_then_the_default() {
    assert_eq!(
        changed_by_for(Some("reviewer"), Some("admin-user"))
            .await
            .as_deref(),
        Some("reviewer")
    );
    assert_eq!(
        changed_by_for(None, Some("admin-user")).await.as_deref(),
        Some("admin-user")
    );
    assert_eq!(
        changed_by_for(None, None).await.as_deref(),
        Some("default_user_id")
    );
    assert_eq!(
        changed_by_for(Some(""), Some("")).await.as_deref(),
        Some("default_user_id")
    );
    assert_eq!(
        changed_by_for(Some(""), Some("admin-user"))
            .await
            .as_deref(),
        Some("admin-user")
    );
}

#[tokio::test]
async fn disabled_audit_logging_spawns_no_task() {
    let (sink, repository) = support::sink(false);
    let audit = SinkCacheSettingsAudit { sink };
    assert!(!audit.enabled());
    let spawner = support::CollectingSpawner::default();
    spawn_cache_settings_audit(
        Arc::new(audit),
        &spawner,
        entry(AuditAction::Created, None, None),
    );
    assert_eq!(spawner.len(), 0);
    assert!(repository.records().is_empty());
}

#[tokio::test]
async fn the_spawned_task_writes_the_entry_without_being_awaited() {
    let audit = support::audit(true);
    let spawner = support::CollectingSpawner::default();
    let erased: Arc<dyn CacheSettingsAudit> = Arc::clone(&audit);
    spawn_cache_settings_audit(
        erased,
        &spawner,
        entry(AuditAction::Created, Some("reviewer"), None),
    );
    assert_eq!(spawner.len(), 1);
    assert_eq!(audit.len(), 0);
    support::run_tasks(&spawner).await;
    let entries = audit.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, AuditAction::Created);
    assert_eq!(entries[0].litellm_changed_by.as_deref(), Some("reviewer"));
}

#[tokio::test]
async fn a_failing_sink_only_warns() {
    let (logs, _guard) = support::capture_logs();
    let audit = support::failing_audit();
    let spawner = support::CollectingSpawner::default();
    let erased: Arc<dyn CacheSettingsAudit> = Arc::clone(&audit);
    spawn_cache_settings_audit(erased, &spawner, entry(AuditAction::Updated, None, None));
    support::run_tasks(&spawner).await;
    assert_eq!(audit.len(), 1);
    assert!(
        logs.text()
            .contains("Failed to write cache-settings audit log: the audit table is unreachable")
    );
}
