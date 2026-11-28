use std::collections::BTreeMap;
use std::error::Error;

use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_secret_managers::secret_contracts::BaseSecretManager;
use secrecy::ExposeSecret as _;

use crate::audit_events::enums::AuditAction;
use crate::audit_events::record::AuditRecordError;
use crate::key_event_sinks::team_params::TeamSecretSettings;

use super::audit::{KeyAuditEntry, KeyEventAudit};
use super::effects::KeyEventEffects;
use super::encode;
use super::error::KeyEventError;
use super::types::{DeletedEvent, KeyEventCaller};

#[derive(Debug, thiserror::Error)]
pub(crate) enum DeletedAuditError {
    #[error("the key row cannot be encoded")]
    Row(#[source] serde_json::Error),
    #[error("the key audit record cannot be built")]
    Audit(#[source] AuditRecordError),
}

impl<Manager> From<DeletedAuditError> for KeyEventError<Manager>
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings>,
{
    fn from(source: DeletedAuditError) -> Self {
        match source {
            DeletedAuditError::Row(source) => Self::Row(source),
            DeletedAuditError::Audit(source) => Self::Audit(source),
        }
    }
}

// KeyManagementEventHooks.create_key_deleted_audit_logs: one scheduled deleted row per key, over whichever row source the caller has
pub(crate) fn create_key_deleted_audit_logs<Rows>(
    audit: &KeyEventAudit,
    keys: Rows,
    caller: &KeyEventCaller,
    litellm_changed_by: Option<&str>,
) -> Result<(), DeletedAuditError>
where
    Rows: IntoIterator<Item = Result<LiteLLMVerificationToken, serde_json::Error>>,
{
    if !audit.settings.is_audit_logging_enabled(None) {
        return Ok(());
    }
    for key in keys {
        let decoded = key.map_err(DeletedAuditError::Row)?;
        let Some(token) = decoded.token.as_ref() else {
            continue;
        };
        let before_value = encode::row_dump(&decoded).map_err(DeletedAuditError::Row)?;
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
            .map_err(DeletedAuditError::Audit)?;
        audit.scheduler.schedule(record);
    }
    Ok(())
}

// async_key_deleted_hook: every audit row is scheduled before the secret-manager delete, which is awaited without a catch because LiteLLM deliberately lets its failure reach the route
pub(super) async fn run<Manager>(
    effects: &KeyEventEffects<'_, Manager>,
    audit: &KeyEventAudit,
    event: &DeletedEvent,
    caller: &KeyEventCaller,
    litellm_changed_by: Option<&str>,
) -> Result<(), KeyEventError<Manager>>
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings> + Send,
    <Manager::Params as TryFrom<TeamSecretSettings>>::Error: Error + Send + Sync + 'static,
{
    create_key_deleted_audit_logs(
        audit,
        event.rows.iter().map(audit_key),
        caller,
        litellm_changed_by,
    )?;
    let rows = event
        .rows
        .iter()
        .map(sink_row::<Manager>)
        .collect::<Result<Vec<_>, KeyEventError<Manager>>>()?;
    effects
        .secrets
        .delete(&rows)
        .await
        .map_err(|source| KeyEventError::Sink { source })?;
    Ok(())
}

fn audit_key(row: &JsonData) -> Result<LiteLLMVerificationToken, serde_json::Error> {
    let value = serde_json::to_value(row)?;
    serde_json::from_value(value)
}

// the secret manager reads only the alias, team and token, so a malformed row it never needs still deletes
fn sink_row<Manager>(row: &JsonData) -> Result<LiteLLMVerificationToken, KeyEventError<Manager>>
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings>,
{
    let value = serde_json::to_value(row).map_err(KeyEventError::Row)?;
    let fields = match value {
        JsonValue::Object(fields) => fields,
        other @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_)) => {
            return serde_json::from_value(other).map_err(KeyEventError::Row);
        }
    };
    let mut seed = BTreeMap::new();
    for name in ["token", "key_alias", "team_id"] {
        if let Some(field) = fields.get(name) {
            seed.insert(name.to_owned(), field.clone());
        }
    }
    let object = serde_json::to_value(seed).map_err(KeyEventError::Row)?;
    serde_json::from_value(object).map_err(KeyEventError::Row)
}
