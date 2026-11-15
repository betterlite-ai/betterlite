use std::borrow::Cow;
use std::future::Future;

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::Guardrail;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::postgres_store::errors::DbError;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use super::error::RegistryError;

const DEFAULT_INFO: &str = "{}";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GuardrailRow {
    pub guardrail_id: String,
    pub guardrail_name: String,
    pub litellm_params: Option<String>,
    pub guardrail_info: Option<String>,
    pub team_id: Option<String>,
    pub status: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewGuardrailRow {
    pub guardrail_id: String,
    pub created_at: DateTime<Utc>,
    pub fields: GuardrailUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailUpdate {
    pub guardrail_name: String,
    pub litellm_params: String,
    pub guardrail_info: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum RowsError {
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("stored guardrail row is not valid JSON")]
    Row {
        #[source]
        source: serde_json::Error,
    },
    #[error("guardrail row is missing")]
    MissingRow,
}

pub trait GuardrailRows: Send + Sync {
    fn create(
        &self,
        row: NewGuardrailRow,
    ) -> impl Future<Output = Result<GuardrailRow, RowsError>> + Send;

    fn delete(&self, guardrail_id: &str) -> impl Future<Output = Result<(), RowsError>> + Send;

    fn update(
        &self,
        guardrail_id: &str,
        row: GuardrailUpdate,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send;

    fn find_active(&self) -> impl Future<Output = Result<Vec<GuardrailRow>, RowsError>> + Send;

    fn find_by_id(
        &self,
        guardrail_id: &str,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send;

    fn find_by_name(
        &self,
        guardrail_name: &str,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send;
}

pub fn row_to_guardrail(row: &GuardrailRow) -> Result<Guardrail, RegistryError> {
    let litellm_params = row_params(row)?;
    let guardrail_info = row_info(row)?;
    Ok(Guardrail {
        guardrail_id: Some(row.guardrail_id.clone()),
        guardrail_name: row.guardrail_name.clone(),
        litellm_params,
        guardrail_info,
        policy_template: None,
        created_at: Some(row.created_at.utc()),
        updated_at: Some(row.updated_at.utc()),
        team_id: row.team_id.clone(),
    })
}

pub fn guardrail_to_new_row(
    guardrail: &Guardrail,
    now: DateTime<Utc>,
) -> Result<NewGuardrailRow, RegistryError> {
    let fields = guardrail_to_update(guardrail, now)?;
    // Prisma's uuid() default: an id in the request never reaches the row
    let guardrail_id = Uuid::new_v4().to_string();
    Ok(NewGuardrailRow {
        guardrail_id,
        created_at: now,
        fields,
    })
}

pub fn guardrail_to_update(
    guardrail: &Guardrail,
    now: DateTime<Utc>,
) -> Result<GuardrailUpdate, RegistryError> {
    let guardrail_name = guardrail.guardrail_name.clone();
    let litellm_params = encode_params(&guardrail.litellm_params)?;
    let guardrail_info = encode_info(guardrail.guardrail_info.as_ref())?;
    Ok(GuardrailUpdate {
        guardrail_name,
        litellm_params,
        guardrail_info,
        updated_at: now,
    })
}

// LiteLLM stores safe_dumps text inside the Json column, but other tools write an object
fn unwrapped_params(text: &str) -> Cow<'_, str> {
    if let Ok(inner) = serde_json::from_str::<String>(text) {
        return Cow::Owned(inner);
    }
    Cow::Borrowed(text)
}

fn row_params(row: &GuardrailRow) -> Result<LitellmParams, RegistryError> {
    let Some(raw) = row.litellm_params.as_deref() else {
        return Err(RegistryError::MissingParams);
    };
    let unwrapped = unwrapped_params(raw);
    decode_litellm_params(unwrapped.as_bytes()).map_err(RegistryError::Params)
}

fn row_info(row: &GuardrailRow) -> Result<Option<OpaqueParams>, RegistryError> {
    let Some(raw) = row.guardrail_info.as_deref() else {
        return Ok(None);
    };
    let unwrapped = unwrapped_params(raw);
    let info = serde_json::from_str(&unwrapped).map_err(|source| RegistryError::Row { source })?;
    Ok(Some(info))
}

fn encode_params(params: &LitellmParams) -> Result<String, RegistryError> {
    let encoded = serde_json::to_string(params);
    let text = encoded.map_err(|source| RegistryError::Encode { source })?;
    Ok(text)
}

fn encode_info(info: Option<&OpaqueParams>) -> Result<String, RegistryError> {
    let Some(stored_info) = info else {
        return Ok(DEFAULT_INFO.to_owned());
    };
    let encoded = serde_json::to_string(stored_info);
    let text = encoded.map_err(|source| RegistryError::Encode { source })?;
    Ok(text)
}
