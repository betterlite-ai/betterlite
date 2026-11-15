use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::audit_events::{LITELLM_STORE_AUDIT_LOGS_ENV, Settings};
use betterlite_config::sections::settings::JsonValue;
use betterlite_server::audit_events::callbacks::{AuditCallbackConfig, AuditCallbackDispatcher};
use betterlite_server::audit_events::enums::{AuditAction, AuditName, AuditTable};
use betterlite_server::audit_events::identity::{CallerIdentity, get_audit_log_changed_by};
use betterlite_server::audit_events::payload::build_audit_log_payload;
use betterlite_server::audit_events::record::{
    AuditLogRecord, decode_audit_record, timestamp_text,
};
use betterlite_server::audit_events::settings::ConfiguredAuditSettings;
use betterlite_server::audit_events::sink::{
    AuditEventSink, AuditLogRepository, AuditRepositoryError, ObjectAuditLog, SystemAuditClock,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;

use super::support::{CollectingSpawner, LocalRecorder, LocalRegistry, delivery_view};
use super::types::{Aspect, CallbackCall, Input, failure};

pub(super) async fn answer(input: &Input) -> crate::helpers::HelperResult {
    match input.aspect {
        Aspect::Policy => policy_answer(input),
        Aspect::Identity => Ok(identity_answer(input)),
        Aspect::Record => Ok(record_view(&decoded_record(&input.record)?)),
        Aspect::Dispatch => dispatch_answer(input).await,
        Aspect::SinkDisabled => sink_answer(input).await,
    }
}

fn policy_answer(call: &Input) -> crate::helpers::HelperResult {
    let lookup = match &call.env {
        Some(env) => BTreeMap::from([(LITELLM_STORE_AUDIT_LOGS_ENV.to_owned(), env.clone())]),
        None => BTreeMap::new(),
    };
    let mut settings = ConfigLoader { lookup }
        .load("{}")
        .map_err(|error| failure("ConfigError", error))?
        .sections
        .audit_events;
    settings.store_audit_logs = call.store;
    Ok(json!({"enabled": settings.is_audit_logging_enabled(call.explicit)}))
}

fn identity_answer(call: &Input) -> JsonValue {
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
    match serde_json::to_vec(record) {
        Ok(bytes) => decode_audit_record(&bytes).map_err(|error| failure("ValidationError", error)),
        Err(error) => Err(failure("InvalidRecord", error)),
    }
}

fn record_view(record: &AuditLogRecord) -> JsonValue {
    let changed_by_api_key = record
        .changed_by_api_key
        .as_ref()
        .map(|api_key| api_key.expose_secret());
    json!({
        "id": record.id,
        "updated_at": timestamp_text(record.updated_at, "Z"),
        "changed_by": record.changed_by,
        "changed_by_api_key": changed_by_api_key,
        "action": record.action.as_str(),
        "table_name": record.table_name.as_str(),
        "object_id": record.object_id,
        "before_value": record.before_value,
        "updated_values": record.updated_values,
    })
}

async fn dispatch_answer(call: &Input) -> crate::helpers::HelperResult {
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

fn local_sink(
    recorder: &Arc<LocalRecorder>,
    repository: &Arc<LocalRepository>,
    spawner: &Arc<CollectingSpawner>,
) -> AuditEventSink {
    AuditEventSink {
        settings: Arc::new(ConfiguredAuditSettings {
            config: Settings {
                store_audit_logs: Some(false),
                store_audit_logs_env: None,
                audit_log_callbacks: Vec::new(),
                s3_audit_callback_params: None,
            },
            callbacks: vec![AuditCallbackConfig::Instance(recorder.clone())],
        }),
        callbacks: Arc::new(AuditCallbackDispatcher::new(
            Arc::new(LocalRegistry),
            spawner.clone(),
        )),
        repository: repository.clone(),
        clock: Arc::new(SystemAuditClock),
        new_id: Arc::new(|| "audit-id-1".to_owned()),
    }
}

fn object_entry<'entry>(
    call: &'entry Input,
    caller: &'entry CallerIdentity,
    action: AuditAction,
    table_name: AuditTable,
) -> ObjectAuditLog<'entry> {
    ObjectAuditLog {
        object_id: &call.object_id,
        action,
        litellm_changed_by: None,
        caller,
        litellm_proxy_admin_name: None,
        table_name,
        before_value: call.before_value.as_deref(),
        after_value: call.after_value.as_deref(),
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

fn stored_ids(repository: &LocalRepository) -> Vec<String> {
    let inserted = repository
        .inserted
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    inserted.iter().map(|record| record.id.clone()).collect()
}

async fn sink_answer(call: &Input) -> crate::helpers::HelperResult {
    let record = decoded_record(&call.record)?;
    let action = AuditAction::parse(&call.action)
        .ok_or_else(|| failure("InvalidInput", "unknown audit action"))?;
    let table_name = AuditTable::parse(&call.table_name)
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
    let sink = local_sink(&recorder, &repository, &spawner);
    sink.create_audit_log_for_update(record).await;
    let caller = CallerIdentity {
        user_id: call.user_id.clone(),
        api_key: call.api_key.clone().map(SecretString::from),
        metadata: None,
        team_metadata: None,
    };
    sink.create_object_audit_log(object_entry(call, &caller, action, table_name))
        .await;
    spawner.run_all().await;
    Ok(json!({
        "delivered": delivery_view(&[recorder]),
        "stored": stored_ids(&repository),
        "calls": 2,
    }))
}
