use std::error::Error;

use betterlite_config::sections::settings::JsonValue;
use betterlite_secret_managers::secret_contracts::BaseSecretManager;
use tracing::warn;

use crate::audit_events::enums::AuditAction;
use crate::key_event_sinks::email::CreatedKey;
use crate::key_event_sinks::team_params::TeamSecretSettings;

use super::audit::{KeyAuditEntry, KeyEventAudit};
use super::effects::{KeyEventEffects, KeyEventEmail};
use super::encode;
use super::error::KeyEventError;
use super::identifiers::{response_key, token_id};
use super::types::{GeneratedEvent, KeyEventCaller};

// async_key_generated_hook: email, then the created audit, then the secret-manager store
pub(super) async fn run<Manager>(
    effects: &KeyEventEffects<'_, Manager>,
    audit: &KeyEventAudit,
    event: &GeneratedEvent,
    caller: &KeyEventCaller,
    litellm_changed_by: Option<&str>,
) -> Result<(), KeyEventError>
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings> + Send,
    <Manager::Params as TryFrom<TeamSecretSettings>>::Error: Error + Send + Sync + 'static,
{
    if matches!(event.data.generate.send_invite_email, Some(Some(true)))
        && let Err(error) = send_created_email(effects.email, event).await
    {
        warn!("Failed to send key created email: {error}");
    }
    if audit.settings.is_audit_logging_enabled(None) {
        let updated_values =
            encode::response_dump(&event.response).map_err(KeyEventError::Encode)?;
        let object_id = event
            .response
            .response
            .token_id
            .as_ref()
            .and_then(Option::as_ref)
            .cloned()
            .unwrap_or_default();
        let record = audit
            .record(KeyAuditEntry {
                action: AuditAction::Created,
                object_id: &object_id,
                changed_by_api_key: caller.identity.api_key.as_ref(),
                before_value: None,
                updated_values: Some(JsonValue::String(updated_values)),
                litellm_changed_by,
                caller: &caller.identity,
            })
            .map_err(KeyEventError::Audit)?;
        audit.scheduler.schedule(record);
    }
    if let Err(error) = store_secret(effects, event).await {
        warn!("Failed to store virtual key in secret manager: {error}");
    }
    Ok(())
}

async fn send_created_email(
    email: &dyn KeyEventEmail,
    event: &GeneratedEvent,
) -> Result<(), KeyEventError> {
    let dump = encode::response_dump(&event.response).map_err(KeyEventError::Encode)?;
    let response: CreatedKey = serde_json::from_str(&dump).map_err(KeyEventError::EmailEvent)?;
    email
        .send_created(response)
        .await
        .map_err(KeyEventError::Email)
}

async fn store_secret<Manager>(
    effects: &KeyEventEffects<'_, Manager>,
    event: &GeneratedEvent,
) -> Result<(), KeyEventError>
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings> + Send,
    <Manager::Params as TryFrom<TeamSecretSettings>>::Error: Error + Send + Sync + 'static,
{
    let secret_name = secret_name(event);
    let team_id = event
        .data
        .base
        .base
        .team_id
        .as_ref()
        .and_then(Option::as_ref)
        .map(String::as_str);
    effects
        .secrets
        .store(&secret_name, response_key(&event.response)?, team_id)
        .await
        .map_err(KeyEventError::sink::<Manager>)?;
    Ok(())
}

// data.key_alias or f"virtual-key-{response.token_id}"
fn secret_name(event: &GeneratedEvent) -> String {
    let alias = event
        .data
        .base
        .base
        .key_alias
        .as_ref()
        .and_then(Option::as_ref);
    match alias {
        Some(name) if !name.is_empty() => name.clone(),
        _ => format!(
            "virtual-key-{}",
            token_id(&event.response).unwrap_or("None")
        ),
    }
}
