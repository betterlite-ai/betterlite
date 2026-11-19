use betterlite_config::sections::audit_events::Settings as AuditPolicy;

use crate::audit::service::AuditService;
use crate::python_hooks::HooksError;
use crate::python_hooks::PythonHooks;
use crate::python_hooks::slots::ListKey;

const DISABLED_WARNING: &str = "'audit_log_callbacks' is configured but audit logging is not enabled. Audit log callbacks will not fire.";

#[derive(Debug, thiserror::Error)]
pub enum AuditStartupError {
    #[error("could not load the audit_log_callbacks entry {entry:?}")]
    Load {
        entry: String,
        #[source]
        source: Box<HooksError>,
    },
}

pub async fn install_audit_callbacks(
    service: &AuditService,
    hooks: &PythonHooks,
    configured: Option<&[String]>,
    policy: &AuditPolicy,
) -> Result<(), AuditStartupError> {
    let Some(configured) = configured else {
        return Ok(());
    };
    // LiteLLM drops the name cache and the list before the first import, so a reload never keeps a stale entry
    service.delivery.named.reset_audit_log_callback_cache();
    service.clear_audit_log_callbacks();
    for entry in configured {
        let callback = hooks
            .load_callback(ListKey::AuditLogCallbacks, entry)
            .await
            .map_err(|source| AuditStartupError::Load {
                entry: entry.to_owned(),
                source: Box::new(source),
            })?;
        service.append_audit_log_callback(callback);
    }
    // LiteLLM passes litellm_settings.store_audit_logs when it is set and the configured policy value otherwise
    if policy.is_audit_logging_enabled(policy.store_audit_logs) {
        tracing::info!(callbacks = ?configured, "Initialized Audit Log Callbacks");
    } else {
        tracing::warn!("{DISABLED_WARNING}");
    }
    Ok(())
}

mod tests;
