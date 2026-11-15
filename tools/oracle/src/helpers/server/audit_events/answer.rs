use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::audit_events::{Settings, environment_flag};
use betterlite_config::sections::settings::JsonValue;
use betterlite_server::audit_events::callbacks::{AuditCallbackConfig, AuditCallbackDispatcher};
use betterlite_server::audit_events::enums::{AuditAction, AuditTable};
use betterlite_server::audit_events::identity::{CallerIdentity, get_audit_log_changed_by};
use betterlite_server::audit_events::payload::build_audit_log_payload;
use betterlite_server::audit_events::record::{
    AuditLogRecord, decode_audit_record, json_timestamp,
};
use betterlite_server::audit_events::settings::AuditSettings;
use betterlite_server::audit_events::sink::{
    AuditClock, AuditEventSink, AuditLogRepository, AuditRepositoryError, ObjectAuditLog,
};
use chrono::{DateTime, FixedOffset};
use serde_json::json;

use super::support::{CollectingSpawner, LocalRecorder, LocalRegistry, delivery_view};
use super::types::{
    CallbackCall, DispatchCall, IdentityCall, ObjectCall, PolicyCall, RecordCall, SinkCall, failure,
};

pub(super) async fn answer(input: &super::types::Input) -> crate::helpers::HelperResult {
    match input {
        super::types::Input::Policy(call) => Ok(policy_answer(call)),
        super::types::Input::Identity(call) => Ok(identity_answer(call)),
        super::types::Input::Record(call) => record_answer(call),
        super::types::Input::Dispatch(call) => dispatch_answer(call).await,
        super::types::Input::SinkDisabled(call) => sink_answer(call).await,
    }
}

fn policy_answer(call: &PolicyCall) -> JsonValue {
    let config = Settings {
        store_audit_logs: call.store,
        store_audit_logs_env: environment_flag(call.env.as_deref()),
        audit_log_callbacks: Vec::new(),
        s3_audit_callback_params: None,
    };
    json!({"enabled": config.is_audit_logging_enabled(call.explicit)})
}

fn identity_answer(call: &IdentityCall) -> JsonValue {
    let caller = CallerIdentity {
        user_id: call.user_id.clone(),
        api_key: None,
        metadata: call.metadata.clone(),
        team_metadata: call.team_metadata.clone(),
    };
    let changed_by = get_audit_log_changed_by(
        call.litellm_changed_by.as_deref(),
        &caller,
        call.admin_name.as_deref(),
    );
    json!({"changed_by": changed_by})
}

fn decoded_record(record: &JsonValue) -> Result<AuditLogRecord, Box<crate::helpers::HelperError>> {
    let bytes = serde_json::to_vec(record).map_err(|error| failure("InvalidRecord", error))?;
    decode_audit_record(&bytes).map_err(|error| failure("ValidationError", error))
}

fn record_view(record: &AuditLogRecord) -> JsonValue {
    json!({
        "id": record.id,
        "updated_at": json_timestamp(record.updated_at),
        "changed_by": record.changed_by,
        "changed_by_api_key": record.changed_by_api_key,
        "action": record.action.as_str(),
        "table_name": record.table_name.as_str(),
        "object_id": record.object_id,
        "before_value": record.before_value,
        "updated_values": record.updated_values,
    })
}

fn record_answer(call: &RecordCall) -> crate::helpers::HelperResult {
    let record = decoded_record(&call.record)?;
    Ok(record_view(&record))
}

async fn dispatch_answer(call: &DispatchCall) -> crate::helpers::HelperResult {
    let record = decoded_record(&call.record)?;
    let spawner = Arc::new(CollectingSpawner::default());
    let mut recorders = Vec::new();
    let mut configs = Vec::new();
    for callback in &call.callbacks {
        match callback {
            CallbackCall::Instance { label, fails } => {
                let recorder = Arc::new(LocalRecorder {
                    label: label.clone(),
                    fails: *fails,
                    delivered: Mutex::new(Vec::new()),
                });
                configs.push(AuditCallbackConfig::Instance(recorder.clone()));
                recorders.push(recorder);
            }
            CallbackCall::Named { name } => {
                configs.push(AuditCallbackConfig::Named(name.clone()));
            }
        }
    }
    let dispatcher = AuditCallbackDispatcher::new(Arc::new(LocalRegistry), spawner.clone());
    dispatcher.dispatch(&configs, None, build_audit_log_payload(&record));
    spawner.run_all().await;
    Ok(json!({"delivered": delivery_view(&recorders)}))
}

struct LocalSettings {
    callbacks: Vec<AuditCallbackConfig>,
}

impl AuditSettings for LocalSettings {
    fn is_audit_logging_enabled(&self, _explicit: Option<bool>) -> bool {
        false
    }

    fn audit_log_callbacks(&self) -> &[AuditCallbackConfig] {
        &self.callbacks
    }

    fn s3_audit_callback_params(&self) -> Option<&JsonValue> {
        None
    }
}

struct LocalRepository {
    inserted: Mutex<Vec<AuditLogRecord>>,
}

#[async_trait]
impl AuditLogRepository for LocalRepository {
    fn available(&self) -> bool {
        true
    }

    async fn insert_audit_log(&self, record: &AuditLogRecord) -> Result<(), AuditRepositoryError> {
        self.inserted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record.clone());
        Ok(())
    }
}

struct LocalClock {
    now: DateTime<FixedOffset>,
}

impl AuditClock for LocalClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.now
    }
}

fn object_audit<'entry>(
    object: &'entry ObjectCall,
    caller: &'entry CallerIdentity,
    action: AuditAction,
    table_name: AuditTable,
) -> ObjectAuditLog<'entry> {
    ObjectAuditLog {
        object_id: &object.object_id,
        action,
        litellm_changed_by: None,
        caller,
        litellm_proxy_admin_name: None,
        table_name,
        before_value: object.before_value.as_deref(),
        after_value: object.after_value.as_deref(),
    }
}

fn stored_ids(repository: &LocalRepository) -> Vec<String> {
    let inserted = repository
        .inserted
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    inserted.iter().map(|record| record.id.clone()).collect()
}

async fn log_object(
    sink: &AuditEventSink,
    call: &SinkCall,
    action: AuditAction,
    table_name: AuditTable,
) {
    let caller = CallerIdentity {
        user_id: call.caller.user_id.clone(),
        api_key: call.caller.api_key.clone(),
        metadata: None,
        team_metadata: None,
    };
    let entry = object_audit(&call.object, &caller, action, table_name);
    sink.create_object_audit_log(entry).await;
}

async fn sink_answer(call: &SinkCall) -> crate::helpers::HelperResult {
    let record = decoded_record(&call.record)?;
    let action = AuditAction::parse(&call.object.action)
        .ok_or_else(|| failure("InvalidInput", "unknown audit action"))?;
    let table_name = AuditTable::parse(&call.object.table_name)
        .ok_or_else(|| failure("InvalidInput", "unknown audit table"))?;
    let recorder = Arc::new(LocalRecorder {
        label: "sink".to_owned(),
        fails: false,
        delivered: Mutex::new(Vec::new()),
    });
    let repository = Arc::new(LocalRepository {
        inserted: Mutex::new(Vec::new()),
    });
    let spawner = Arc::new(CollectingSpawner::default());
    let sink = AuditEventSink {
        settings: Arc::new(LocalSettings {
            callbacks: vec![AuditCallbackConfig::Instance(recorder.clone())],
        }),
        callbacks: Arc::new(AuditCallbackDispatcher::new(
            Arc::new(LocalRegistry),
            spawner.clone(),
        )),
        repository: repository.clone(),
        clock: Arc::new(LocalClock {
            now: record.updated_at,
        }),
        new_id: Arc::new(|| "audit-id-1".to_owned()),
    };
    sink.create_audit_log_for_update(record).await;
    log_object(&sink, call, action, table_name).await;
    spawner.run_all().await;
    Ok(json!({
        "delivered": delivery_view(&[recorder]),
        "stored": stored_ids(&repository),
        "calls": 2,
    }))
}
