use betterlite_auth::identity::{
    ActiveState, AgentIdentityBinding, ExecutionMode, IdentityProvider,
};
use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::{AgentCard, AgentObjectPermission};
use betterlite_db::postgres_store::diagnostics::DriverError;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::de::DeserializeOwned;
use tokio_postgres::Row;
use uuid::Uuid;

use super::columns::{
    optional_json, optional_text, optional_text_array, required_boolean, required_json,
    required_text,
};
use super::{ColumnFailure, RowDecodeError};

pub(super) fn decode_identity(row: &Row) -> Result<Option<AgentIdentityBinding>, RowDecodeError> {
    let Some(agent_id) = optional_text(row, "i_agent_id")? else {
        return Ok(None);
    };
    let last_authenticated_at: Option<NaiveDateTime> = row
        .try_get("i_last_authenticated_at")
        .map_err(|driver| RowDecodeError::Timestamp {
            source: ColumnFailure::Driver(DriverError { driver }),
        })?;
    let binding = AgentIdentityBinding {
        agent_id,
        active: ActiveState(required_boolean(row, "i_active")?),
        provider: decode_identity_provider(row)?,
        tenant_id: decode_uuid(row, "i_tenant_id")?,
        client_id: decode_uuid(row, "i_client_id")?,
        service_principal_id: decode_optional_uuid(row, "i_service_principal_id")?,
        issuer: required_text(row, "i_issuer")?,
        required_roles: optional_text_array(row, "i_required_roles")?.unwrap_or_default(),
        required_scopes: optional_text_array(row, "i_required_scopes")?.unwrap_or_default(),
        revision: required_text(row, "i_revision")?,
        last_authenticated_at: last_authenticated_at
            .map(|naive| DateTime::from_naive_utc_and_offset(naive, Utc)),
    };
    Ok(Some(binding))
}

fn decode_identity_provider(row: &Row) -> Result<IdentityProvider, RowDecodeError> {
    let stored = required_text(row, "i_provider")?;
    serde_json::from_value(AgentJson::String(stored)).map_err(|source| RowDecodeError::Identity {
        source: ColumnFailure::Json(source),
    })
}

fn decode_uuid(row: &Row, name: &str) -> Result<Uuid, RowDecodeError> {
    let stored = required_text(row, name)?;
    Uuid::parse_str(&stored).map_err(|source| RowDecodeError::Identity {
        source: ColumnFailure::Uuid(source),
    })
}

fn decode_optional_uuid(row: &Row, name: &str) -> Result<Option<Uuid>, RowDecodeError> {
    let Some(stored) = optional_text(row, name)? else {
        return Ok(None);
    };
    Uuid::parse_str(&stored)
        .map(Some)
        .map_err(|source| RowDecodeError::Identity {
            source: ColumnFailure::Uuid(source),
        })
}

pub(super) fn decode_object_permission(
    row: &Row,
    object_permission_id: Option<&str>,
) -> Result<Option<AgentObjectPermission>, RowDecodeError> {
    if object_permission_id.is_none() {
        return Ok(None);
    }
    let mcp_servers = optional_text_array(row, "p_mcp_servers")?;
    let mcp_access_groups = optional_text_array(row, "p_mcp_access_groups")?;
    let mcp_toolsets = optional_text_array(row, "p_mcp_toolsets")?;
    let mcp_tool_permissions = json_object_column(row, "p_mcp_tool_permissions")?;
    let models = optional_text_array(row, "p_models")?;
    let agents = optional_text_array(row, "p_agents")?;
    Ok(Some(AgentObjectPermission {
        mcp_servers: Some(mcp_servers),
        mcp_access_groups: Some(mcp_access_groups),
        mcp_toolsets: Some(mcp_toolsets),
        mcp_tool_permissions: Some(mcp_tool_permissions),
        models: Some(models),
        agents: Some(agents),
    }))
}

pub(super) fn decode_agent_card(row: &Row) -> Result<AgentCard, RowDecodeError> {
    let stored = required_json(row, "agent_card_params")?;
    parse_stored_json(&stored).map_err(|source| RowDecodeError::AgentCard {
        source: ColumnFailure::Json(source),
    })
}

pub(super) fn decode_execution_mode(row: &Row) -> Result<ExecutionMode, RowDecodeError> {
    let stored = required_text(row, "execution_mode")?;
    serde_json::from_value(AgentJson::String(stored)).map_err(|source| {
        RowDecodeError::ExecutionMode {
            source: ColumnFailure::Json(source),
        }
    })
}

pub(super) fn json_object_column<T>(row: &Row, name: &str) -> Result<Option<T>, RowDecodeError>
where
    T: DeserializeOwned,
{
    let Some(stored) = optional_json(row, name)? else {
        return Ok(None);
    };
    parse_stored_json(&stored)
        .map(Some)
        .map_err(|source| RowDecodeError::Json {
            source: ColumnFailure::Json(source),
        })
}

fn parse_stored_json<T>(stored: &AgentJson) -> Result<T, serde_json::Error>
where
    T: DeserializeOwned,
{
    match stored {
        AgentJson::String(text) => serde_json::from_str(text),
        AgentJson::Null
        | AgentJson::Bool(_)
        | AgentJson::Number(_)
        | AgentJson::Array(_)
        | AgentJson::Object(_) => serde_json::from_value(stored.clone()),
    }
}
