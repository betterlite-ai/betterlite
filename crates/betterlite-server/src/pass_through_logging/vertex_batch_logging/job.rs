use betterlite_models::decisions::Json;
use betterlite_vertex::provider_metadata::timestamp::LocalTimeContext;
use betterlite_vertex::vertex_batch_records::response::to_batch_object;
use betterlite_vertex::vertex_batch_records::types::VertexBatchPredictionResponse;
use jiff::tz::TimeZone;

use super::error::VertexBatchError;
use super::record::{Outcome, Pending};
use super::{
    BATCH_PREDICTION_JOBS_ROUTE, SUCCESS_STATUS, UNKNOWN_MODEL, VertexBatchInput,
    VertexBatchLogging,
};
use crate::pass_through_logging::anthropic_batch::unified_object_id;
use crate::pass_through_logging::batch_attribution::is_collection_route;

pub(super) fn handle(
    logging: &VertexBatchLogging,
    input: &VertexBatchInput<'_>,
) -> Result<Outcome, VertexBatchError> {
    let body: Json =
        serde_json::from_slice(input.body).map_err(|source| VertexBatchError::Parse { source })?;
    let Json::Object(fields) = body else {
        return Ok(Outcome::Rejected(input.status_code));
    };
    if input.status_code != SUCCESS_STATUS || !fields.contains_key("name") {
        return Ok(Outcome::Rejected(input.status_code));
    }
    let job: VertexBatchPredictionResponse =
        serde_json::from_slice(input.body).map_err(|source| VertexBatchError::Job { source })?;
    // createTime ends in Z, so it is read as UTC where LiteLLM reads it in the process's zone
    let utc = LocalTimeContext {
        zone: TimeZone::UTC,
    };
    let batch_object =
        to_batch_object(&job, &utc).map_err(|source| VertexBatchError::Transform { source })?;
    let model = job_model(fields.get("model"))?;
    let batch_id = batch_object.id.clone();
    let actual_model_id = logging.get_actual_model_id_from_router(&model);
    let unified = unified_object_id(&actual_model_id, &batch_id);
    let is_batch_create = is_collection_route(input.url_route, BATCH_PREDICTION_JOBS_ROUTE);
    logging.store_batch_managed_object(
        &unified,
        batch_object,
        input.request_metadata,
        is_batch_create,
    );
    Ok(Outcome::Pending(Pending {
        batch_id,
        model,
        unified_object_id: unified,
    }))
}

fn job_model(model: Option<&Json>) -> Result<String, VertexBatchError> {
    let kind = match model {
        None | Some(Json::Null) => return Ok(UNKNOWN_MODEL.to_owned()),
        Some(Json::Text(name)) => return Ok(name.clone()),
        Some(Json::Bool(_)) => "a boolean",
        Some(Json::Number(_)) => "a number",
        Some(Json::List(_)) => "a list",
        Some(Json::Object(_)) => "an object",
    };
    Err(VertexBatchError::ModelNotText { kind })
}
