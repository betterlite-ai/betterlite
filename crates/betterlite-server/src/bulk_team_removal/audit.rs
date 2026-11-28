use betterlite_db::identity_rows::key::LiteLLMVerificationToken;

use super::error::BulkRemovalError;
use crate::key_events::audit::KeyEventAudit;
use crate::key_events::deleted::{DeletedAuditError, create_key_deleted_audit_logs};
use crate::key_events::types::KeyEventCaller;

pub trait DeletedKeyAudit: Send + Sync {
    fn schedule_deleted(
        &self,
        keys: &[LiteLLMVerificationToken],
        caller: &KeyEventCaller,
    ) -> Result<(), BulkRemovalError>;
}

impl From<DeletedAuditError> for BulkRemovalError {
    fn from(source: DeletedAuditError) -> Self {
        match source {
            DeletedAuditError::Row(failure) => Self::Encode(failure),
            DeletedAuditError::Audit(failure) => Self::Audit(failure),
        }
    }
}

// the removal's caller and litellm_changed_by None go to B784's create_key_deleted_audit_logs, which owns the row shape and the enabled check
pub struct KeyEventDeletedAudit<'deps> {
    pub audit: &'deps KeyEventAudit,
}

impl DeletedKeyAudit for KeyEventDeletedAudit<'_> {
    fn schedule_deleted(
        &self,
        keys: &[LiteLLMVerificationToken],
        caller: &KeyEventCaller,
    ) -> Result<(), BulkRemovalError> {
        create_key_deleted_audit_logs(
            self.audit,
            keys.iter()
                .cloned()
                .map(Ok::<LiteLLMVerificationToken, serde_json::Error>),
            caller,
            None,
        )?;
        Ok(())
    }
}
