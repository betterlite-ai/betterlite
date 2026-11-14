use betterlite_models::decisions::Json;
use uuid::Uuid;

use super::configuration::{IdentitySource, configuration_failure};
use super::identity_write::identity_write;
use super::write_fields::ManagedWriteFields;
use crate::agent_policy::{AgentExecutionMode, AgentResponse};
use crate::identity::{AgentIdentityFailure, EntraIdentityConfig, FailureCode};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum IdentityUpdate {
    #[default]
    Absent,
    Null,
    Value(Json),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ManagedWriteInput {
    pub identity: IdentityUpdate,
    pub execution_mode: Option<AgentExecutionMode>,
    pub enabled: Option<bool>,
}

pub fn managed_write_fields(
    input: &ManagedWriteInput,
    existing: Option<&AgentResponse>,
    next_revision: fn() -> Uuid,
) -> Result<ManagedWriteFields, AgentIdentityFailure> {
    let identity = match &input.identity {
        IdentityUpdate::Absent | IdentityUpdate::Null => None,
        IdentityUpdate::Value(value) => Some(parse_identity_config(value)?),
    };
    let identity_present = !matches!(&input.identity, IdentityUpdate::Absent);
    let existing_mode = existing.map(|agent| agent.execution_mode.clone());
    let mode = input
        .execution_mode
        .clone()
        .or(existing_mode)
        .unwrap_or_default();
    let current = if identity_present {
        identity.as_ref().map(IdentitySource::Configuration)
    } else {
        existing
            .and_then(|agent| agent.identity.as_ref())
            .map(IdentitySource::Binding)
    };
    let enabling_without_binding = input.enabled == Some(true)
        && !identity_present
        && existing.is_some_and(|agent| agent.identity_managed);
    if let Some(failure) = configuration_failure(current, &mode, enabling_without_binding) {
        return Err(failure);
    }
    let mut fields = if identity_present {
        identity_write(identity.as_ref(), existing, next_revision)
    } else {
        ManagedWriteFields::default()
    };
    fields.enabled = fields.enabled.or(input.enabled);
    fields.execution_mode = fields
        .execution_mode
        .or_else(|| input.execution_mode.as_ref().map(|_| mode));
    Ok(fields)
}

fn parse_identity_config(value: &Json) -> Result<EntraIdentityConfig, AgentIdentityFailure> {
    let parsed = serde_json::to_string(value).and_then(|text| serde_json::from_str(&text));
    match parsed {
        Ok(configuration) => Ok(configuration),
        Err(source) => Err(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: format!("Invalid agent identity configuration: {source}"),
        }),
    }
}
