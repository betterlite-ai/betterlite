use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::{
    BatchObject, BatchObjectKind, BatchStatus, RequestCounts,
};
use indexmap::IndexMap;

use crate::batch_status::handler::{
    EpochInput, extract_job_id_from_arn, predict_output_file_uri, record_counts_from_response,
    to_epoch,
};
use crate::batch_status::timestamps::parse_iso_timestamp;

use super::error::BatchCallError;
use super::fields::{falsey_text, matching_time, text_field};

pub const ENDPOINT: &str = "/v1/chat/completions";
pub const COMPLETION_WINDOW: &str = "24h";

// GetModelInvocationJob answers a job record, which the handler projects onto an OpenAI batch
pub fn project_job(response: &Json, batch_id: &str) -> Result<BatchObject, BatchCallError> {
    let Json::Object(fields) = response else {
        return Err(BatchCallError::ResponseShape);
    };
    JobRecord::read(fields, batch_id)?.into_batch(fields)
}

struct JobRecord {
    arn: String,
    status: BatchStatus,
    input_uri: String,
    output_prefix: String,
    output_file: Option<String>,
    created_at: i64,
    in_progress_at: Option<i64>,
    end_time: Option<i64>,
    request_counts: Option<RequestCounts>,
}

impl JobRecord {
    fn read(fields: &IndexMap<String, Json>, batch_id: &str) -> Result<Self, BatchCallError> {
        let arn = job_arn(fields, batch_id)?;
        let input_uri = nested_uri(fields, "inputDataConfig", "s3InputDataConfig")?;
        let output_prefix = nested_uri(fields, "outputDataConfig", "s3OutputDataConfig")?;
        let job_id = extract_job_id_from_arn(&arn);
        Ok(Self {
            output_file: predict_output_file_uri(&output_prefix, &input_uri, job_id.as_deref()),
            status: job_status(fields),
            created_at: time_field(fields, "submitTime")?.unwrap_or(0),
            in_progress_at: time_field(fields, "lastModifiedTime")?,
            end_time: time_field(fields, "endTime")?,
            request_counts: record_counts_from_response(fields)?,
            arn,
            input_uri,
            output_prefix,
        })
    }

    fn into_batch(self, fields: &IndexMap<String, Json>) -> Result<BatchObject, BatchCallError> {
        let metadata = metadata(fields, &self)?;
        Ok(BatchObject {
            id: self.arn,
            completion_window: COMPLETION_WINDOW.to_owned(),
            created_at: self.created_at,
            endpoint: ENDPOINT.to_owned(),
            input_file_id: self.input_uri,
            object: BatchObjectKind::Batch,
            status: self.status,
            cancelled_at: matching_time(self.status, BatchStatus::Cancelled, self.end_time),
            cancelling_at: None,
            completed_at: matching_time(self.status, BatchStatus::Completed, self.end_time),
            error_file_id: None,
            errors: None,
            expired_at: matching_time(self.status, BatchStatus::Expired, self.end_time),
            expires_at: None,
            failed_at: matching_time(self.status, BatchStatus::Failed, self.end_time),
            finalizing_at: None,
            in_progress_at: self.in_progress_at,
            metadata: Some(metadata),
            model: None,
            output_file_id: output_file(self.status, self.output_file),
            request_counts: self.request_counts,
            usage: None,
            provider_fields: <_>::default(),
        })
    }
}

fn job_arn(fields: &IndexMap<String, Json>, batch_id: &str) -> Result<String, BatchCallError> {
    match fields.get("jobArn") {
        None => Ok(batch_id.to_owned()),
        Some(Json::Text(arn)) => Ok(arn.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(BatchCallError::Field { field: "jobArn" })
        }
    }
}

// str(response.get("status", "")) keeps a number's digits and leaves any unknown word in progress
fn job_status(fields: &IndexMap<String, Json>) -> BatchStatus {
    let name = match fields.get("status") {
        None => String::new(),
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null) => String::from("None"),
        Some(Json::Bool(flag)) => if *flag { "True" } else { "False" }.to_owned(),
        Some(Json::Number(number)) => number.to_string(),
        Some(Json::List(_) | Json::Object(_)) => String::new(),
    };
    match name.as_str() {
        "Submitted" | "Validating" | "Scheduled" => BatchStatus::Validating,
        "InProgress" => BatchStatus::InProgress,
        "Stopping" => BatchStatus::Cancelling,
        "Stopped" => BatchStatus::Cancelled,
        "Completed" | "PartiallyCompleted" => BatchStatus::Completed,
        "Failed" => BatchStatus::Failed,
        "Expired" => BatchStatus::Expired,
        _ => BatchStatus::InProgress,
    }
}

// the handler walks inputDataConfig.s3InputDataConfig.s3Uri with a {} default at each level
fn nested_uri(
    fields: &IndexMap<String, Json>,
    outer: &'static str,
    inner: &'static str,
) -> Result<String, BatchCallError> {
    let outer_config = match fields.get(outer) {
        None => return Ok(String::new()),
        Some(Json::Object(config)) => config,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(BatchCallError::Field { field: outer });
        }
    };
    let inner_config = match outer_config.get(inner) {
        None => return Ok(String::new()),
        Some(Json::Object(config)) => config,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(BatchCallError::Field { field: inner });
        }
    };
    match inner_config.get("s3Uri") {
        None => Ok(String::new()),
        Some(Json::Text(uri)) => Ok(uri.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(BatchCallError::Field { field: "s3Uri" })
        }
    }
}

// botocore turns a wire timestamp into a datetime before the handler reads it, and a number
// keeps its digits as botocore's own epoch conversion does
fn time_field(
    fields: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<i64>, BatchCallError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => parse_iso_timestamp(text)
            .map(Some)
            .ok_or(BatchCallError::Field { field }),
        Some(value @ (Json::Bool(_) | Json::Number(_))) => {
            to_epoch(Some(EpochInput::Json(value))).map_err(BatchCallError::from)
        }
        Some(Json::List(_) | Json::Object(_)) => Err(BatchCallError::Field { field }),
    }
}

fn metadata(
    fields: &IndexMap<String, Json>,
    job: &JobRecord,
) -> Result<IndexMap<String, String>, BatchCallError> {
    let mut metadata = IndexMap::with_capacity(7);
    metadata.insert(
        "model_arn".to_owned(),
        text_field(fields, "modelId")?.unwrap_or_default(),
    );
    metadata.insert("job_arn".to_owned(), job.arn.clone());
    metadata.insert(
        "job_name".to_owned(),
        text_field(fields, "jobName")?.unwrap_or_default(),
    );
    metadata.insert(
        "failure_message".to_owned(),
        falsey_text(fields, "message")?,
    );
    metadata.insert("input_s3_uri".to_owned(), job.input_uri.clone());
    metadata.insert("output_s3_uri".to_owned(), job.output_prefix.clone());
    metadata.insert(
        "output_file_uri".to_owned(),
        job.output_file.clone().unwrap_or_default(),
    );
    Ok(metadata)
}

fn output_file(status: BatchStatus, uri: Option<String>) -> Option<String> {
    if status == BatchStatus::Completed {
        return uri;
    }
    None
}
