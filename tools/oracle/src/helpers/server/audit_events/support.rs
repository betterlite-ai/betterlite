use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_server::audit_events::callbacks::{
    AuditCallbackError, AuditCallbackFuture, AuditCallbackRegistry, AuditLogCallback,
    AuditTaskSpawner,
};
use betterlite_server::audit_events::payload::AuditLogPayload;
use serde_json::json;

pub(super) struct LocalRecorder {
    pub(super) label: String,
    pub(super) fails: bool,
    pub(super) delivered: Mutex<Vec<AuditLogPayload>>,
}

#[async_trait]
impl AuditLogCallback for LocalRecorder {
    async fn async_log_audit_log_event(
        &self,
        payload: AuditLogPayload,
    ) -> Result<(), AuditCallbackError> {
        if self.fails {
            return Err(AuditCallbackError::Failed);
        }
        self.delivered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(payload);
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct CollectingSpawner {
    tasks: Mutex<Vec<AuditCallbackFuture>>,
}

impl CollectingSpawner {
    pub(super) async fn run_all(&self) {
        let tasks: Vec<AuditCallbackFuture> = self
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect();
        for task in tasks {
            task.await;
        }
    }
}

impl AuditTaskSpawner for CollectingSpawner {
    fn spawn(&self, task: AuditCallbackFuture) {
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(task);
    }
}

pub(super) struct LocalRegistry;

impl AuditCallbackRegistry for LocalRegistry {
    fn resolve(&self, _name: &str) -> Option<Arc<dyn AuditLogCallback>> {
        None
    }

    fn build_dedicated_s3_logger(
        &self,
        _params: &JsonValue,
    ) -> Result<Arc<dyn AuditLogCallback>, AuditCallbackError> {
        Err(AuditCallbackError::Failed)
    }
}

fn stored_json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap_or(JsonValue::Null)
}

fn payload_view(payload: &AuditLogPayload) -> JsonValue {
    json!({
        "id": payload.id,
        "updated_at": payload.updated_at,
        "changed_by": payload.changed_by,
        "changed_by_api_key": payload.hashed_changed_by_api_key,
        "action": payload.action,
        "table_name": payload.table_name,
        "object_id": payload.object_id,
        "before_value": payload.before_value.as_deref().map(stored_json),
        "updated_values": payload.updated_values.as_deref().map(stored_json),
    })
}

pub(super) fn delivery_view(recorders: &[Arc<LocalRecorder>]) -> JsonValue {
    let mut delivered = Vec::new();
    for recorder in recorders {
        let payloads = recorder
            .delivered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for payload in payloads {
            delivered.push(json!({
                "callback": recorder.label,
                "payload": payload_view(&payload),
            }));
        }
    }
    JsonValue::Array(delivered)
}
