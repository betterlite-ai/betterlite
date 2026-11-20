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

#[must_use]
pub fn ingestion_jobs_url(region: &str, knowledge_base_id: &str, data_source_id: &str) -> String {
    format!(
        "{}/knowledgebases/{}/datasources/{}/ingestionjobs/",
        bedrock_agent_endpoint(region),
        super::encode_segment(knowledge_base_id),
        super::encode_segment(data_source_id)
    )
}

#[must_use]
pub fn ingestion_job_url(
    region: &str,
    knowledge_base_id: &str,
    data_source_id: &str,
    job_id: &str,
) -> String {
    format!(
        "{}{}",
        ingestion_jobs_url(region, knowledge_base_id, data_source_id),
        super::encode_segment(job_id)
    )
}

pub fn start_ingestion_job(
    region: &str,
    knowledge_base_id: &str,
    data_source_id: &str,
) -> Result<CallRequest, CallError> {
    let request = StartIngestionJobRequest {
        client_token: Uuid::new_v4().to_string(),
    };
    let body = serde_json::to_vec(&request).map_err(|source| CallError::Serialize {
        operation: START_INGESTION_JOB,
        source,
    })?;
    let url = ingestion_jobs_url(region, knowledge_base_id, data_source_id);
    Ok(agent_call(region, START_INGESTION_JOB, "PUT", url, body))
}

pub fn get_ingestion_job(
    region: &str,
    knowledge_base_id: &str,
    data_source_id: &str,
    job_id: &str,
) -> CallRequest {
    let url = ingestion_job_url(region, knowledge_base_id, data_source_id, job_id);
    agent_call(region, GET_INGESTION_JOB, "GET", url, Vec::new())
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
