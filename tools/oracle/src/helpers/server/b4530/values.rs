use betterlite_db::budget_permission_rows::JsonData;
use betterlite_server::memory_records::models::{
    CreateRequest, DeleteResponse, ListResponse, MemoryRow, UpdateRequest,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{ModelKind, RequestKind, Step, StepFailure};

// memory_endpoints.py create_memory and upsert_memory: the row fields each handler builds
pub(super) fn request_value(step: &Step) -> Result<Value, StepFailure> {
    let body = serde_json::to_value(&step.body)?;
    match step.request {
        Some(RequestKind::Create) => {
            let request: CreateRequest = serde_json::from_value(body)?;
            let column = metadata_column(request.metadata_column()?)?;
            Ok(json!({
                "key": request.key,
                "value": request.value,
                "metadata": request.metadata,
                "user_id": request.user_id,
                "team_id": request.team_id,
                "metadata_column": column,
            }))
        }
        Some(RequestKind::Update) => {
            let request: UpdateRequest = serde_json::from_value(body)?;
            let sent = request.fields_sent();
            let column = metadata_column(request.metadata_column()?)?;
            Ok(json!({
                "fields_sent": sent,
                "value": request.value(),
                "metadata_written": request.metadata.is_some(),
                "metadata": request.metadata.clone().flatten(),
                "metadata_column": column,
            }))
        }
        None => Ok(Value::Null),
    }
}

// the JSON text the metadata column stores, read back as the value the case compares
fn metadata_column(text: Option<String>) -> Result<Option<JsonData>, StepFailure> {
    match text {
        Some(json_text) => Ok(Some(serde_json::from_str(&json_text)?)),
        None => Ok(None),
    }
}

pub(super) fn model_value(step: &Step) -> Result<Value, StepFailure> {
    match step.model {
        Some(ModelKind::Row) => serialized::<MemoryRow>(&step.body),
        Some(ModelKind::List) => serialized::<ListResponse>(&step.body),
        Some(ModelKind::Delete) => serialized::<DeleteResponse>(&step.body),
        None => Ok(Value::Null),
    }
}

// types/memory_management.py: the response model exactly as its JSON encoder writes it
fn serialized<T>(body: &JsonData) -> Result<Value, StepFailure>
where
    T: DeserializeOwned + Serialize,
{
    let wire = serde_json::to_value(body)?;
    let model: T = serde_json::from_value(wire)?;
    Ok(serde_json::to_value(model)?)
}
