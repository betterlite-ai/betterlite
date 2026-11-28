use betterlite_config::sections::settings::JsonValue;
use betterlite_db::model_secrets::DEFAULT_USER_ID;
use betterlite_redact::setting_redaction::dump_redacted_config;

use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::identity::CallerIdentity;
use crate::audit_events::sink::{AuditEventSink, ObjectAuditLog};

const ENVIRONMENT_VARIABLES: &str = "environment_variables";

pub struct ConfigChange<'entry> {
    pub param: &'entry str,
    pub action: AuditAction,
    pub table: AuditTable,
    pub before: Option<&'entry JsonValue>,
    pub after: Option<&'entry JsonValue>,
    pub caller: &'entry CallerIdentity,
}

// proxy_server.py's create_config_audit_log: a settings write never fails because its audit row could not be written
pub async fn config_audit_log(sink: &AuditEventSink, change: ConfigChange<'_>) {
    let redact_all = change.param == ENVIRONMENT_VARIABLES;
    let before = dump_redacted_config(change.before, redact_all);
    let after = dump_redacted_config(change.after, redact_all);
    sink.create_object_audit_log(ObjectAuditLog {
        object_id: change.param,
        action: change.action,
        litellm_changed_by: None,
        caller: change.caller,
        litellm_proxy_admin_name: Some(DEFAULT_USER_ID),
        table_name: change.table,
        before_value: before.as_deref(),
        after_value: after.as_deref(),
    })
    .await;
}

mod tests;
