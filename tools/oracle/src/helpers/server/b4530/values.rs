use betterlite_db::budget_permission_rows::JsonData;
use betterlite_server::memory_records::error::internal_error;
use betterlite_server::memory_records::models::{
    CreateRequest, DeleteResponse, ListResponse, MemoryRow, UpdateRequest,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::inputs::{CaseError, InternalErrorValue};
use super::{ModelKind, RequestKind, Step, StepFailure};

// memory_endpoints.py create_memory: the row fields plus the JSON text the metadata column stores
pub(super) fn create_value(step: &Step) -> Result<Value, StepFailure> {
    let create: CreateRequest = serde_json::from_value(serde_json::to_value(&step.body)?)?;
    let mut column: Option<JsonData> = None;
    if let Some(text) = create.metadata_column()? {
        column = Some(serde_json::from_str(&text)?);
    }
    Ok(json!({
        "key": create.key,
        "value": create.value,
        "metadata": create.metadata,
        "user_id": create.user_id,
        "team_id": create.team_id,
        "metadata_column": column,
    }))
}

// memory_endpoints.py upsert_memory: field presence decides whether the metadata column is written
pub(super) fn update_value(step: &Step) -> Result<Value, StepFailure> {
    let update: UpdateRequest = serde_json::from_value(serde_json::to_value(&step.body)?)?;
    let mut column: Option<JsonData> = None;
    if let Some(text) = update.metadata_column()? {
        column = Some(serde_json::from_str(&text)?);
    }
    Ok(json!({
        "fields_sent": update.fields_sent(),
        "value": update.value(),
        "metadata_written": update.metadata.is_some(),
        "metadata": update.metadata.clone().flatten(),
        "metadata_column": column,
    }))
}

// memory_endpoints.py _internal_error: the 500 and its generic detail, with the cause logged
pub(super) fn internal_error_value(step: &Step) -> Result<Value, StepFailure> {
    let source = CaseError {
        message: step.message.clone(),
    };
    let error = internal_error(&step.log_message, &source, &step.detail);
    let value = InternalErrorValue {
        status: error.status.as_u16(),
        detail: error.detail,
    };
    Ok(serde_json::to_value(value)?)
}

pub(super) fn request_value(step: &Step) -> Result<Value, StepFailure> {
    match step.request {
        Some(RequestKind::Create) => create_value(step),
        Some(RequestKind::Update) => update_value(step),
        None => Err(StepFailure::Input),
    }
}

pub(super) fn model_value(step: &Step) -> Result<Value, StepFailure> {
    match step.model {
        Some(ModelKind::Row) => response_value::<MemoryRow>(step),
        Some(ModelKind::List) => response_value::<ListResponse>(step),
        Some(ModelKind::Delete) => response_value::<DeleteResponse>(step),
        None => Err(StepFailure::Input),
    }
}

// types/memory_management.py: the response model exactly as its JSON encoder writes it
fn response_value<T>(step: &Step) -> Result<Value, StepFailure>
where
    T: DeserializeOwned + Serialize,
{
    let model: T = serde_json::from_value(serde_json::to_value(&step.body)?)?;
    Ok(serde_json::to_value(model)?)
}
