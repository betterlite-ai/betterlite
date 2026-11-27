use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::{
    BatchObject, BatchObjectKind, BatchStatus, RequestCounts,
};
use indexmap::IndexMap;
use serde_json::value::RawValue;

use crate::bedrock_embedding_io::transport::ClientFactory;
use crate::embedding_jobs::logging::StatusLogger;
use crate::embedding_jobs::status::{
    AsyncInvokeStatusCall, StatusServices, get_async_invoke_status,
};
use crate::embedding_jobs::transport::StatusTransport;
use crate::image_call::plan::CredentialSource;

use super::error::BatchCallError;
use super::fields::{
    falsey_text, matching_time, optional_integer, required_integer, required_object,
    required_optional_integer, required_text,
};

pub const ENDPOINT: &str = "/v1/embeddings";
pub const COMPLETION_WINDOW: &str = "24h";

pub struct AsyncStatusCall<'source> {
    pub invocation_arn: &'source str,
    pub region: &'source str,
    pub optional_params: &'source mut BTreeMap<String, Box<RawValue>>,
    pub environment: &'source BTreeMap<String, String>,
    pub now: SystemTime,
}

// Twelve Labs async invoke: GetAsyncInvoke answers the job record, which the handler projects
// onto the OpenAI batch shape with a fixed single-request count
pub async fn retrieve_async_status<Source, Factory>(
    call: AsyncStatusCall<'_>,
    services: &StatusServices<'_, Source, Factory>,
    log: Option<&mut StatusLogger<'_>>,
) -> Result<BatchObject, BatchCallError>
where
    Source: CredentialSource + Sync,
    Factory: ClientFactory + Sync,
    Factory::Client: StatusTransport,
{
    let response = get_async_invoke_status(
        AsyncInvokeStatusCall {
            invocation_arn: call.invocation_arn,
            region: call.region,
            optional_params: call.optional_params,
            environment: call.environment,
            now: call.now,
        },
        services,
        log,
    )
    .await?;
    project_async_status(&response)
}

pub fn project_async_status(response: &Json) -> Result<BatchObject, BatchCallError> {
    let Json::Object(fields) = response else {
        return Err(BatchCallError::ResponseShape);
    };
    Ok(AsyncRecord::read(fields)?.into_batch())
}

struct AsyncRecord {
    id: String,
    status: BatchStatus,
    created_at: i64,
    in_progress_at: Option<i64>,
    end_time: Option<i64>,
    metadata: IndexMap<String, String>,
}

impl AsyncRecord {
    fn read(fields: &IndexMap<String, Json>) -> Result<Self, BatchCallError> {
        let mut metadata = IndexMap::with_capacity(3);
        metadata.insert("output_file_id".to_owned(), output_uri(fields)?);
        metadata.insert(
            "failure_message".to_owned(),
            falsey_text(fields, "failureMessage")?,
        );
        metadata.insert("model_arn".to_owned(), required_text(fields, "modelArn")?);
        Ok(Self {
            id: required_text(fields, "invocationArn")?,
            status: async_status(fields)?,
            created_at: required_integer(fields, "submitTime")?,
            in_progress_at: required_optional_integer(fields, "lastModifiedTime")?,
            end_time: optional_integer(fields, "endTime")?,
            metadata,
        })
    }

    fn into_batch(self) -> BatchObject {
        BatchObject {
            id: self.id,
            completion_window: COMPLETION_WINDOW.to_owned(),
            created_at: self.created_at,
            endpoint: ENDPOINT.to_owned(),
            input_file_id: String::new(),
            object: BatchObjectKind::Batch,
            status: self.status,
            cancelled_at: None,
            cancelling_at: None,
            completed_at: self.end_time,
            error_file_id: None,
            errors: None,
            expired_at: None,
            expires_at: None,
            failed_at: matching_time(self.status, BatchStatus::Failed, self.end_time),
            finalizing_at: None,
            in_progress_at: self.in_progress_at,
            metadata: Some(self.metadata),
            model: None,
            output_file_id: None,
            request_counts: Some(RequestCounts {
                completed: i64::from(self.status == BatchStatus::Completed),
                failed: i64::from(self.status == BatchStatus::Failed),
                total: 1,
            }),
            usage: None,
            provider_fields: <_>::default(),
        }
    }
}

// the raw httpx body carries the job's own status word, which LiteLLMBatch validates against
// the OpenAI batch statuses
fn async_status(fields: &IndexMap<String, Json>) -> Result<BatchStatus, BatchCallError> {
    let name = required_text(fields, "status")?;
    match name.as_str() {
        "validating" => Ok(BatchStatus::Validating),
        "failed" => Ok(BatchStatus::Failed),
        "in_progress" => Ok(BatchStatus::InProgress),
        "finalizing" => Ok(BatchStatus::Finalizing),
        "completed" => Ok(BatchStatus::Completed),
        "expired" => Ok(BatchStatus::Expired),
        "cancelling" => Ok(BatchStatus::Cancelling),
        "cancelled" => Ok(BatchStatus::Cancelled),
        other => Err(BatchCallError::InvalidStatus {
            status: other.to_owned(),
        }),
    }
}

// status_response["outputDataConfig"]["s3OutputDataConfig"]["s3Uri"], required at every step
fn output_uri(fields: &IndexMap<String, Json>) -> Result<String, BatchCallError> {
    let outer = required_object(fields, "outputDataConfig")?;
    let inner = required_object(outer, "s3OutputDataConfig")?;
    required_text(inner, "s3Uri")
}
