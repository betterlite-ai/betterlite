use std::sync::Arc;

use betterlite_config::audit::Settings as AuditSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::audit::callbacks::NamedAuditCallbacks;
use betterlite_logging::audit::delivery::{AuditCallback, AuditDelivery};

use super::callbacks::AuditCallbackEntry;
use super::python;
use crate::audit_events::payload::build_audit_log_payload;
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::sink::AuditLogRepository;

pub(super) struct AuditWriteTarget<'audit> {
    pub settings: &'audit AuditSettings,
    pub entries: &'audit [AuditCallbackEntry],
    pub delivery: &'audit AuditDelivery,
    pub repository: &'audit dyn AuditLogRepository,
}

// create_audit_log_for_update: recheck the policy, deliver to every callback, then insert and never fail the caller
pub(super) async fn write_audit_log(target: AuditWriteTarget<'_>, record: AuditLogRecord) {
    if !target.settings.config.is_audit_logging_enabled(None) {
        return;
    }
    tracing::debug!("creating audit log for {record:?}");
    let payload = build_audit_log_payload(&record);
    let loggers = selected_loggers(
        target.entries,
        &target.delivery.named,
        target.settings.config.s3_audit_callback_params.as_ref(),
    );
    let dispatched = target.delivery.dispatch(&loggers, &payload);
    tracing::debug!(?dispatched, "the audit log callbacks were dispatched");
    if !target.repository.available() {
        tracing::error!("prisma_client is None, cannot write audit log to DB");
        return;
    }
    if let Err(error) = target.repository.insert_audit_log(&record).await {
        tracing::error!("Failed Creating audit log {error}");
    }
}

// the configured list keeps its order; an unresolved name is warned about and skipped by the named cache
fn selected_loggers(
    entries: &[AuditCallbackEntry],
    named: &NamedAuditCallbacks,
    s3_params: Option<&JsonValue>,
) -> Vec<AuditCallback> {
    let mut loggers: Vec<AuditCallback> = Vec::new();
    for entry in entries {
        match entry {
            AuditCallbackEntry::Logger(logger) => loggers.push(Arc::clone(logger)),
            AuditCallbackEntry::Named(name) => {
                if let Some(logger) = named.resolve(name, s3_params) {
                    loggers.push(logger);
                }
            }
            AuditCallbackEntry::Python(hook) => {
                if let Some(logger) = python::audit_logger(Arc::clone(hook)) {
                    loggers.push(logger);
                }
            }
        }
    }
    loggers
}
