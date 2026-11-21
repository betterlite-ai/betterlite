#![cfg(test)]

use serde::Deserialize;

use super::{
    GET_INGESTION_JOB, IngestionJobCall, START_INGESTION_JOB, ingestion_job, ingestion_job_call,
    ingestion_job_id,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartBody {
    client_token: String,
}

#[test]
fn call_start_ingestion_job_puts_to_the_data_source_jobs_path() {
    let call = ingestion_job_call("us-west-2", "kb1", "ds1", IngestionJobCall::Start).unwrap();
    assert_eq!(call.operation, START_INGESTION_JOB);
    assert_eq!(call.service, "bedrock");
    assert_eq!(call.signing_region, "us-west-2");
    assert_eq!(call.method, "PUT");
    assert_eq!(
        call.url,
        "https://bedrock-agent.us-west-2.amazonaws.com/knowledgebases/kb1/datasources/ds1/ingestionjobs/"
    );
    let content_type = call
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("Content-Type"))
        .map(|(_, value)| value.clone());
    assert_eq!(content_type.as_deref(), Some("application/json"));
    let body: StartBody = serde_json::from_slice(&call.body).unwrap();
    uuid::Uuid::parse_str(&body.client_token).unwrap();
}

#[test]
fn call_get_ingestion_job_gets_the_job_path() {
    let call =
        ingestion_job_call("us-west-2", "kb1", "ds1", IngestionJobCall::Get("job1")).unwrap();
    assert_eq!(call.operation, GET_INGESTION_JOB);
    assert_eq!(call.method, "GET");
    assert_eq!(
        call.url,
        "https://bedrock-agent.us-west-2.amazonaws.com/knowledgebases/kb1/datasources/ds1/ingestionjobs/job1"
    );
    assert!(call.body.is_empty());
    assert!(call.headers.is_empty());
}

#[test]
fn call_ingestion_job_paths_encode_each_identifier() {
    let call =
        ingestion_job_call("us-west-2", "kb/1", "ds 1", IngestionJobCall::Get("job#1")).unwrap();
    assert!(call.url.contains("knowledgebases/kb%2F1"));
    assert!(call.url.contains("datasources/ds%201"));
    assert!(call.url.ends_with("ingestionjobs/job%231"));
}

#[test]
fn call_ingestion_job_reads_the_status_statistics_and_failure_reasons() {
    let body = br#"{"ingestionJob":{"ingestionJobId":"job1","status":"COMPLETE","statistics":{"numberOfNewDocumentsIndexed":3,"numberOfDocumentsScanned":4},"failureReasons":["a"]}}"#;
    let job = ingestion_job(body, GET_INGESTION_JOB).unwrap();
    assert_eq!(job.status.as_deref(), Some("COMPLETE"));
    assert_eq!(
        job.statistics
            .and_then(|statistics| statistics.new_documents_indexed),
        Some(3)
    );
    assert_eq!(job.failure_reasons, Some(vec!["a".to_owned()]));
}

#[test]
fn call_ingestion_job_reads_the_job_id() {
    let body = br#"{"ingestionJob":{"ingestionJobId":"job1","status":"STARTING"}}"#;
    assert_eq!(ingestion_job_id(body, START_INGESTION_JOB).unwrap(), "job1");
}

#[test]
fn call_ingestion_job_reports_a_missing_job_id() {
    let body = br#"{"ingestionJob":{"status":"STARTING"}}"#;
    ingestion_job_id(body, START_INGESTION_JOB).unwrap_err();
}

#[test]
fn call_ingestion_job_reports_a_body_that_is_not_json() {
    ingestion_job(b"nope", GET_INGESTION_JOB).unwrap_err();
}
