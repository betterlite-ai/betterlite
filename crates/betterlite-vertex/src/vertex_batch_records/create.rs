use betterlite_openai::api::batch_records::CreateBatchRequest;
use uuid::Uuid;

use crate::vertex_batch_records::error::BatchRecordsError;
use crate::vertex_batch_records::model::batch_job_model;
use crate::vertex_batch_records::types::{
    GcsDestination, GcsSource, InputConfig, OutputConfig, VertexAIBatchPredictionJob,
};

const INSTANCES_FORMAT: &str = "jsonl";
const PREDICTIONS_FORMAT: &str = "jsonl";
const DISPLAY_NAME_PREFIX: &str = "litellm-vertex-batch-";

pub fn to_batch_job(
    request: &CreateBatchRequest,
    project: Option<&str>,
    location: Option<&str>,
    display_id: Uuid,
) -> Result<VertexAIBatchPredictionJob, BatchRecordsError> {
    let input_file_id = request
        .input_file_id
        .as_deref()
        .ok_or(BatchRecordsError::InputFileIdRequired)?;
    let model = batch_job_model(input_file_id, project, location)?;
    Ok(VertexAIBatchPredictionJob {
        display_name: format!("{DISPLAY_NAME_PREFIX}{display_id}"),
        model,
        input_config: InputConfig {
            instances_format: INSTANCES_FORMAT.to_owned(),
            gcs_source: GcsSource {
                uris: vec![input_file_id.to_owned()],
            },
        },
        output_config: OutputConfig {
            predictions_format: PREDICTIONS_FORMAT.to_owned(),
            gcs_destination: GcsDestination {
                output_uri_prefix: output_prefix(input_file_id),
            },
        },
    })
}

// LiteLLM cuts the input uri at its last slash, so an id without one is its own output prefix
fn output_prefix(input_file_id: &str) -> String {
    let Some((prefix, _file)) = input_file_id.rsplit_once('/') else {
        return input_file_id.to_owned();
    };
    prefix.to_owned()
}
