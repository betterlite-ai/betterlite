use std::error::Error;

use betterlite_config::sections::settings::JsonValue;
use betterlite_secret_managers::secret_contracts::BaseSecretManager;
use secrecy::ExposeSecret as _;
use tracing::warn;

use crate::audit_events::enums::AuditAction;
use crate::audit_events::record::AuditLogRecord;
use crate::key_event_sinks::team_params::TeamSecretSettings;
use crate::key_fields::shared::hash_token_if_needed;

use super::audit::{KeyAuditEntry, KeyEventAudit};
use super::effects::KeyEventEffects;
use super::encode;
use super::error::KeyEventError;
use super::identifiers::stored_name;
use super::types::{KeyEventCaller, UpdatedEvent};

// async_key_updated_hook: the rename, then the updated audit; the route's response is never read
pub(super) async fn run<Manager>(
    effects: &KeyEventEffects<'_, Manager>,
    audit: &KeyEventAudit,
    event: &UpdatedEvent,
    caller: &KeyEventCaller,
    litellm_changed_by: Option<&str>,
) -> Result<(), KeyEventError>
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings> + Send,
    <Manager::Params as TryFrom<TeamSecretSettings>>::Error: Error + Send + Sync + 'static,
{
    if let Err(error) = rename(effects, event).await {
        warn!("Failed to rename virtual key in secret manager: {error}");
    }
    if audit.settings.is_audit_logging_enabled(None) {
        audit_update(audit, event, caller, litellm_changed_by)?;
    }
    Ok(())
}

async fn rename<Manager>(
    effects: &KeyEventEffects<'_, Manager>,
    event: &UpdatedEvent,
) -> Result<(), KeyEventError>
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings> + Send,
    <Manager::Params as TryFrom<TeamSecretSettings>>::Error: Error + Send + Sync + 'static,
{
    let Some(alias) = event
        .data
        .base
        .base
        .key_alias
        .as_ref()
        .and_then(Option::as_ref)
    else {
        return Ok(());
    };
    if Some(alias.as_str()) == event.row.key_alias.as_deref() {
        return Ok(());
    }
    effects
        .secrets
        .rename(
            &stored_name(&event.row),
            alias,
            event.row.team_id.as_deref(),
        )
        .await?;
    Ok(())
}

fn audit_update(
    audit: &KeyEventAudit,
    event: &UpdatedEvent,
    caller: &KeyEventCaller,
    litellm_changed_by: Option<&str>,
) -> Result<(), KeyEventError> {
    let key = event
        .data
        .base
        .key_fields
        .key
        .as_ref()
        .and_then(Option::as_ref)
        .ok_or(KeyEventError::MissingUpdateKey)?;
    let object_id = hash_token_if_needed(key.expose_secret());
    let updated_values = encode::update_dump(&event.data).map_err(KeyEventError::Encode)?;
    let before_value = encode::row_python_dump(&event.row).map_err(KeyEventError::Row)?;
    let mut record = audit
        .record(KeyAuditEntry {
            action: AuditAction::Updated,
            object_id: &object_id,
            changed_by_api_key: caller.identity.api_key.as_ref(),
            before_value: Some(JsonValue::String(before_value)),
            updated_values: Some(JsonValue::String(updated_values)),
            litellm_changed_by,
            caller: &caller.identity,
        })
        .map_err(KeyEventError::Audit)?;
    if matches!(event.data.update.project_id, Some(None)) {
        restore_project_null(&mut record);
    }
    audit.scheduler.schedule(record);
    Ok(())
}

// the masked values were a dict before the explicit null detach is put back into them
fn restore_project_null(record: &mut AuditLogRecord) {
    let Some(JsonValue::Object(values)) = &mut record.updated_values else {
        return;
    };
    values.insert("project_id".to_owned(), JsonValue::Null);
}
