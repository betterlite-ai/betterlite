use std::error::Error;

use betterlite_config::sections::settings::JsonValue;
use betterlite_secret_managers::secret_contracts::BaseSecretManager;
use secrecy::ExposeSecret as _;
use tracing::warn;

use crate::audit_events::enums::AuditAction;
use crate::key_event_sinks::team_params::TeamSecretSettings;

use super::audit::{KeyAuditEntry, KeyEventAudit};
use super::effects::{KeyEventEffects, KeyEventEmail};
use super::encode;
use super::error::KeyEventError;
use super::identifiers::{response_key, stored_name, token_id};
use super::types::{KeyEventCaller, RotatedEvent};

// async_key_rotated_hook: the rotation, then the rotated email, then the rotated audit
pub(super) async fn run<Manager>(
    effects: &KeyEventEffects<'_, Manager>,
    audit: &KeyEventAudit,
    event: &RotatedEvent,
    caller: &KeyEventCaller,
    litellm_changed_by: Option<&str>,
) -> Result<(), KeyEventError>
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings> + Send,
    <Manager::Params as TryFrom<TeamSecretSettings>>::Error: Error + Send + Sync + 'static,
{
    if token_id(&event.response).is_some() {
        if let Err(error) = rotate(effects, event).await {
            warn!("Failed to rotate virtual key in secret manager: {error}");
        }
    }
    if let Err(error) = send_rotated_email(effects.email, event).await {
        warn!("Failed to send key rotated email: {error}");
    }
    if audit.settings.is_audit_logging_enabled(None) && event.row.token.is_some() {
        audit_rotated(audit, event, caller, litellm_changed_by)?;
    }
    Ok(())
}

async fn rotate<Manager>(
    effects: &KeyEventEffects<'_, Manager>,
    event: &RotatedEvent,
) -> Result<(), KeyEventError>
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings> + Send,
    <Manager::Params as TryFrom<TeamSecretSettings>>::Error: Error + Send + Sync + 'static,
{
    let initial = stored_name(&event.row);
    let new_name = new_name(event, &initial);
    effects
        .secrets
        .rotate(
            &initial,
            &new_name,
            response_key(&event.response)?,
            event.row.team_id.as_deref(),
        )
        .await
        .map_err(KeyEventError::sink)?;
    Ok(())
}

// response.key_alias or requested alias or the current name, each by Python truthiness
fn new_name(event: &RotatedEvent, initial: &str) -> String {
    let from_response = event
        .response
        .base
        .base
        .key_alias
        .as_ref()
        .and_then(Option::as_ref)
        .filter(|alias| !alias.is_empty());
    let requested = event
        .data
        .as_ref()
        .and_then(|data| data.base.base.key_alias.as_ref().and_then(Option::as_ref))
        .filter(|alias| !alias.is_empty());
    let Some(alias) = from_response.or(requested) else {
        return initial.to_owned();
    };
    alias.clone()
}

async fn send_rotated_email(
    email: &dyn KeyEventEmail,
    event: &RotatedEvent,
) -> Result<(), KeyEventError> {
    let dump = encode::response_dump(&event.response).map_err(KeyEventError::Encode)?;
    email
        .send_rotated(&dump, event.row.key_alias.as_deref())
        .await
        .map_err(KeyEventError::Email)
}

fn audit_rotated(
    audit: &KeyEventAudit,
    event: &RotatedEvent,
    caller: &KeyEventCaller,
    litellm_changed_by: Option<&str>,
) -> Result<(), KeyEventError> {
    let Some(token) = event.row.token.as_ref() else {
        return Ok(());
    };
    let updated_values = encode::response_dump(&event.response).map_err(KeyEventError::Encode)?;
    let before_value = encode::row_dump(&event.row).map_err(KeyEventError::Row)?;
    let record = audit
        .record(KeyAuditEntry {
            action: AuditAction::Rotated,
            object_id: token.expose_secret(),
            changed_by_api_key: caller.token.as_ref(),
            before_value: Some(JsonValue::String(before_value)),
            updated_values: Some(JsonValue::String(updated_values)),
            litellm_changed_by,
            caller: &caller.identity,
        })
        .map_err(KeyEventError::Audit)?;
    audit.scheduler.schedule(record);
    Ok(())
}
