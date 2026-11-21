use percent_encoding::utf8_percent_encode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::CallRequest;
use super::error::CallError;
use crate::bedrock_ingestion::client::BEDROCK_AGENT_SERVICE;
use crate::bedrock_ingestion::client::endpoints::bedrock_agent_endpoint;

pub const START_INGESTION_JOB: &str = "StartIngestionJob";
pub const GET_INGESTION_JOB: &str = "GetIngestionJob";

const JSON_CONTENT_TYPE: &str = "application/json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartIngestionJobRequest {
    client_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestionJobResponse {
    pub ingestion_job: IngestionJob,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestionJob {
    pub ingestion_job_id: Option<String>,
    pub status: Option<String>,
    pub statistics: Option<IngestionStatistics>,
    pub failure_reasons: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestionStatistics {
    #[serde(rename = "numberOfNewDocumentsIndexed")]
    pub new_documents_indexed: Option<i64>,
}

// the jobs collection when no job id is given, and one job's own URL when it is
#[must_use]
pub fn ingestion_job_url(
    region: &str,
    knowledge_base_id: &str,
    data_source_id: &str,
    job_id: Option<&str>,
) -> String {
    let mut segments = vec![
        "knowledgebases".to_owned(),
        utf8_percent_encode(knowledge_base_id, super::UNRESERVED).to_string(),
        "datasources".to_owned(),
        utf8_percent_encode(data_source_id, super::UNRESERVED).to_string(),
        "ingestionjobs".to_owned(),
    ];
    if let Some(id) = job_id {
        segments.push(utf8_percent_encode(id, super::UNRESERVED).to_string());
    }
    let path = segments.join("/");
    let suffix = if job_id.is_some() { "" } else { "/" };
    format!("{}/{path}{suffix}", bedrock_agent_endpoint(region))
}

// StartIngestionJob carries a client token in its body, GetIngestionJob only names the job
#[derive(Debug, Clone, Copy)]
pub enum IngestionJobCall<'job> {
    Start,
    Get(&'job str),
}

pub fn ingestion_job_call(
    region: &str,
    knowledge_base_id: &str,
    data_source_id: &str,
    call: IngestionJobCall<'_>,
) -> Result<CallRequest, CallError> {
    let (operation, method, job_id) = match call {
        IngestionJobCall::Start => (START_INGESTION_JOB, "PUT", None),
        IngestionJobCall::Get(job_id) => (GET_INGESTION_JOB, "GET", Some(job_id)),
    };
    let body = if job_id.is_some() {
        Vec::new()
    } else {
        let request = StartIngestionJobRequest {
            client_token: Uuid::new_v4().to_string(),
        };
        serde_json::to_vec(&request).map_err(|source| CallError::Serialize { operation, source })?
    };
    let url = ingestion_job_url(region, knowledge_base_id, data_source_id, job_id);
    Ok(agent_call(region, operation, method, url, body))
}

pub fn ingestion_job_id(body: &[u8], operation: &'static str) -> Result<String, CallError> {
    let job = ingestion_job(body, operation)?;
    job.ingestion_job_id.ok_or(CallError::MissingEntry {
        operation,
        field: "ingestionJob.ingestionJobId",
    })
}

pub fn ingestion_job(body: &[u8], operation: &'static str) -> Result<IngestionJob, CallError> {
    let response: IngestionJobResponse =
        serde_json::from_slice(body).map_err(|source| CallError::Json { operation, source })?;
    Ok(response.ingestion_job)
}

fn agent_call(
    region: &str,
    operation: &'static str,
    method: &'static str,
    url: String,
    body: Vec<u8>,
) -> CallRequest {
    let headers = if body.is_empty() {
        Vec::new()
    } else {
        vec![("Content-Type".to_owned(), JSON_CONTENT_TYPE.to_owned())]
    };
    CallRequest {
        operation,
        service: BEDROCK_AGENT_SERVICE,
        signing_region: region.to_owned(),
        method,
        url,
        headers,
        body,
    }
}

mod tests;
