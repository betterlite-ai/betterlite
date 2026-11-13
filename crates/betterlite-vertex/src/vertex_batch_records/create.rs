use betterlite_openai::api::batch_records::CreateBatchRequest;
use uuid::Uuid;

use crate::vertex_batch_records::error::BatchRecordsError;
use crate::vertex_batch_records::model::batch_job_model;
use crate::vertex_batch_records::types::{
    GcsDestination, GcsSource, InputConfig, OutputConfig, VertexAIBatchPredictionJob,
};

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
    let (prefix, _file) = input_file_id
        .rsplit_once('/')
        .unwrap_or((input_file_id, ""));
    Ok(VertexAIBatchPredictionJob {
        display_name: format!("litellm-vertex-batch-{display_id}"),
        model,
        input_config: InputConfig {
            instances_format: "jsonl".to_owned(),
            gcs_source: GcsSource {
                uris: vec![input_file_id.to_owned()],
            },
        },
        output_config: OutputConfig {
            predictions_format: "jsonl".to_owned(),
            gcs_destination: GcsDestination {
                output_uri_prefix: prefix.to_owned(),
            },
        },
    })
}
