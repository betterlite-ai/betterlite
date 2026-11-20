use betterlite_auth::identity::{ActiveState, ExecutionMode};
use betterlite_core::agents::AgentResponse;
use betterlite_core::agents::json::AgentJson;
use betterlite_db::postgres_store::diagnostics::DriverError;
use betterlite_db::postgres_store::rows::RowCodec;
use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio_postgres::Row;

use self::columns::{
    optional_integer, optional_json, optional_text, optional_text_array, required_boolean,
    required_text, required_timestamp, spend_value,
};
use self::values::{
    decode_agent_card, decode_execution_mode, decode_identity, decode_object_permission,
    json_object_column,
};
use crate::agent_credentials::kill_switch::parse_agent_kill_switch;
use crate::agent_credentials::params::parse_agent_litellm_params;

mod columns;
mod tests;
mod values;

pub struct StoredAgent {
    pub agent: AgentResponse,
    pub object_permission_id: Option<String>,
    pub litellm_params_column: Option<AgentJson>,
    pub kill_switch_column: Option<AgentJson>,
}

pub struct AgentRowCodec;

#[derive(Debug, Error)]
pub enum ColumnFailure {
    #[error("database driver error")]
    Driver(#[source] DriverError),
    #[error("the stored JSON is not readable")]
    Json(#[source] serde_json::Error),
    #[error("the stored value is not a UUID")]
    Uuid(#[source] uuid::Error),
    #[error("the stored value has the wrong shape")]
    Shape,
}

#[derive(Debug, Error)]
pub enum RowDecodeError {
    #[error("an agent text column is unreadable")]
    Text(#[source] ColumnFailure),
    #[error("an agent text array column is unreadable")]
    TextArray(#[source] ColumnFailure),
    #[error("an agent integer column is unreadable")]
    Integer(#[source] ColumnFailure),
    #[error("an agent boolean column is unreadable")]
    Boolean(#[source] ColumnFailure),
    #[error("an agent timestamp column is unreadable")]
    Timestamp(#[source] ColumnFailure),
    #[error("the agent spend column is unreadable")]
    Spend(#[source] ColumnFailure),
    #[error("an agent JSON column is unreadable")]
    Json(#[source] ColumnFailure),
    #[error("the agent_card_params column is not an agent card")]
    AgentCard(#[source] ColumnFailure),
    #[error("the execution_mode column is not a known mode")]
    ExecutionMode(#[source] ColumnFailure),
    #[error("the agent identity row is unreadable")]
    Identity(#[source] ColumnFailure),
}

impl RowCodec for AgentRowCodec {
    type Model = StoredAgent;
    type Error = RowDecodeError;

    fn decode(&self, row: &Row) -> Result<Option<StoredAgent>, RowDecodeError> {
        decode_stored_agent(row).map(Some)
    }
}

fn decode_stored_agent(row: &Row) -> Result<StoredAgent, RowDecodeError> {
    let object_permission_id: Option<String> = optional_text(row, "object_permission_id")?;
    let litellm_params_column: Option<AgentJson> = optional_json(row, "litellm_params")?;
    let kill_switch_column: Option<AgentJson> = optional_json(row, "kill_switch")?;
    let agent = decode_agent(
        row,
        object_permission_id.as_deref(),
        litellm_params_column.as_ref(),
        kill_switch_column.as_ref(),
    )?;
    Ok(StoredAgent {
        agent,
        object_permission_id,
        litellm_params_column,
        kill_switch_column,
    })
}

fn decode_agent(
    row: &Row,
    object_permission_id: Option<&str>,
    litellm_params_column: Option<&AgentJson>,
    kill_switch_column: Option<&AgentJson>,
) -> Result<AgentResponse, RowDecodeError> {
    let created_at = Some(DateTime::from_naive_utc_and_offset(
        required_timestamp(row, "created_at")?,
        Utc,
    ));
    let updated_at = Some(DateTime::from_naive_utc_and_offset(
        required_timestamp(row, "updated_at")?,
        Utc,
    ));
    Ok(AgentResponse {
        identity: decode_identity(row)?,
        identity_managed: required_boolean(row, "identity_managed")?,
        enabled: ActiveState(required_boolean(row, "enabled")?),
        execution_mode: decode_execution_mode(row)?,
        jwt_auth_configured: false,
        agent_id: required_text(row, "agent_id")?,
        agent_name: required_text(row, "agent_name")?,
        litellm_params: litellm_params_column.map(|value| parse_agent_litellm_params(Some(value))),
        agent_card_params: decode_agent_card(row)?,
        object_permission: decode_object_permission(row, object_permission_id)?,
        spend: Some(spend_value(row)?),
        tpm_limit: optional_integer(row, "tpm_limit")?.map(i64::from),
        rpm_limit: optional_integer(row, "rpm_limit")?.map(i64::from),
        session_tpm_limit: optional_integer(row, "session_tpm_limit")?.map(i64::from),
        session_rpm_limit: optional_integer(row, "session_rpm_limit")?.map(i64::from),
        static_headers: json_object_column(row, "static_headers")?,
        extra_headers: optional_text_array(row, "extra_headers")?,
        access_group_ids: optional_text_array(row, "access_group_ids")?,
        kill_switch: parse_agent_kill_switch(kill_switch_column),
        keys: None,
        search_score: None,
        created_at,
        updated_at,
        created_by: Some(required_text(row, "created_by")?),
        updated_by: Some(required_text(row, "updated_by")?),
    })
}
