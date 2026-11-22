use std::collections::BTreeMap;
use std::error::Error;

use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_secret_managers::secret_contracts::BaseSecretManager;
use secrecy::ExposeSecret as _;

use crate::audit_events::enums::AuditAction;
use crate::key_event_sinks::team_params::TeamSecretSettings;

use super::audit::{KeyAuditEntry, KeyEventAudit};
use super::effects::KeyEventEffects;
use super::encode;
use super::error::KeyEventError;
use super::types::{DeletedEvent, KeyEventCaller};

// async_key_deleted_hook: every audit row is scheduled before the secret-manager delete, which is awaited without a catch because LiteLLM deliberately lets its failure reach the route
pub(super) async fn run<Manager>(
    effects: &KeyEventEffects<'_, Manager>,
    audit: &KeyEventAudit,
    event: &DeletedEvent,
    caller: &KeyEventCaller,
    litellm_changed_by: Option<&str>,
) -> Result<(), KeyEventError>
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings> + Send,
    <Manager::Params as TryFrom<TeamSecretSettings>>::Error: Error + Send + Sync + 'static,
{
    if audit.settings.is_audit_logging_enabled(None) {
        audit_rows(audit, event, caller, litellm_changed_by)?;
    }
    let rows = event
        .rows
        .iter()
        .map(sink_row)
        .collect::<Result<Vec<_>, KeyEventError>>()?;
    effects.secrets.delete(&rows).await?;
    Ok(())
}

fn audit_rows(
    audit: &KeyEventAudit,
    event: &DeletedEvent,
    caller: &KeyEventCaller,
    litellm_changed_by: Option<&str>,
) -> Result<(), KeyEventError> {
    for row in &event.rows {
        let value = serde_json::to_value(row).map_err(KeyEventError::Row)?;
        let key: LiteLLMVerificationToken =
            serde_json::from_value(value).map_err(KeyEventError::Row)?;
        let Some(token) = key.token.as_ref() else {
            continue;
        };
        let before_value = encode::row_dump(&key).map_err(KeyEventError::Row)?;
        let record = audit
            .record(KeyAuditEntry {
                action: AuditAction::Deleted,
                object_id: token.expose_secret(),
                changed_by_api_key: caller.token.as_ref(),
                before_value: Some(JsonValue::String(before_value)),
                updated_values: Some(JsonValue::String("{}".to_owned())),
                litellm_changed_by,
                caller: &caller.identity,
            })
            .map_err(KeyEventError::Audit)?;
        audit.scheduler.schedule(record);
    }
    Ok(())
}

// the secret manager reads only the alias, team and token, so a malformed row it never needs still deletes
fn sink_row(row: &JsonData) -> Result<LiteLLMVerificationToken, KeyEventError> {
    let value = serde_json::to_value(row).map_err(KeyEventError::Row)?;
    match value {
        JsonValue::Object(fields) => {
            let mut seed = BTreeMap::new();
            for name in ["token", "key_alias", "team_id"] {
                if let Some(field) = fields.get(name) {
                    seed.insert(name.to_owned(), field.clone());
                }
            }
            let object = serde_json::to_value(seed).map_err(KeyEventError::Row)?;
            serde_json::from_value(object).map_err(KeyEventError::Row)
        }
        other => serde_json::from_value(other).map_err(KeyEventError::Row),
    }
}
