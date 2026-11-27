use betterlite_config::sections::settings::JsonValue;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use secrecy::ExposeSecret as _;

use super::error::BulkRemovalError;
use crate::audit_events::enums::AuditAction;
use crate::key_events::audit::{KeyAuditEntry, KeyEventAudit};
use crate::key_events::encode::row_dump;
use crate::key_events::types::KeyEventCaller;

pub trait DeletedKeyAudit: Send + Sync {
    fn schedule_deleted(
        &self,
        keys: &[LiteLLMVerificationToken],
        caller: &KeyEventCaller,
    ) -> Result<(), BulkRemovalError>;
}

// KeyManagementEventHooks.create_key_deleted_audit_logs: schedule one deleted row per key, never the secret-manager delete
pub struct KeyEventDeletedAudit<'deps> {
    pub audit: &'deps KeyEventAudit,
}

impl DeletedKeyAudit for KeyEventDeletedAudit<'_> {
    fn schedule_deleted(
        &self,
        keys: &[LiteLLMVerificationToken],
        caller: &KeyEventCaller,
    ) -> Result<(), BulkRemovalError> {
        if !self.audit.settings.is_audit_logging_enabled(None) {
            return Ok(());
        }
        for key in keys {
            let Some(token) = key.token.as_ref() else {
                continue;
            };
            let before_value = row_dump(key)?;
            let record = self.audit.record(KeyAuditEntry {
                action: AuditAction::Deleted,
                object_id: token.expose_secret(),
                changed_by_api_key: caller.token.as_ref(),
                before_value: Some(JsonValue::String(before_value)),
                updated_values: Some(JsonValue::String("{}".to_owned())),
                litellm_changed_by: None,
                caller: &caller.identity,
            })?;
            self.audit.scheduler.schedule(record);
        }
        Ok(())
    }
}
