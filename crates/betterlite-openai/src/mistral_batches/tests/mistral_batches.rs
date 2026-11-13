#![cfg(test)]

use crate::api::batch_records::BatchEndpoint;
use crate::batches::records::{BatchObject, BatchStatus};
use crate::mistral_batches::error::MistralBatchesError;
use crate::mistral_batches::records::{
    MistralBatchError, MistralBatchJob, MistralBatchStatus, MistralGetRequest,
};
use crate::mistral_batches::transformation::{
    create_batch, openai_status, retrieve_batch, to_batch_object,
};
use crate::mistral_identity::error::MistralIdentityError;
use betterlite_config::sections::mistral_resources::Settings;
use http::Method;
use rstest::rstest;
use secrecy::SecretString;
use std::collections::BTreeMap;

fn settings(api_base: Option<&str>, api_key: Option<&str>) -> Settings {
    Settings {
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(SecretString::from),
    }
}

fn job(text: &str) -> MistralBatchJob {
    serde_json::from_str(text).expect("a native job")
}

fn data(text: &str) -> crate::api::batch_records::CreateBatchRequest {
    serde_json::from_str(text).expect("a create batch request")
}

#[rstest]
#[case("QUEUED", BatchStatus::Validating)]
#[case("RUNNING", BatchStatus::InProgress)]
#[case("SUCCESS", BatchStatus::Completed)]
#[case("FAILED", BatchStatus::Failed)]
#[case("TIMEOUT_EXCEEDED", BatchStatus::Expired)]
#[case("CANCELLATION_REQUESTED", BatchStatus::Cancelling)]
#[case("CANCELLED", BatchStatus::Cancelled)]
fn job_statuses_map_onto_openai_statuses(#[case] native: &str, #[case] expected: BatchStatus) {
    let parsed: MistralBatchStatus = serde_json::from_value(serde_json::json!(native)).unwrap();
    assert_eq!(openai_status(parsed), expected);
}

#[test]
fn create_sends_one_input_file_the_endpoint_and_the_model() {
    let plan = create_batch(
        &data(r#"{"input_file_id": "file-1", "endpoint": "/v1/chat/completions"}"#),
        "mistral-small-latest",
        None,
        &settings(None, None),
    )
    .expect("a plan");
    assert_eq!(plan.url, "https://api.mistral.ai/v1/batch/jobs");
    assert_eq!(plan.body.input_files, vec![String::from("file-1")]);
    assert_eq!(plan.body.endpoint, BatchEndpoint::ChatCompletions);
    assert_eq!(plan.body.model, "mistral-small-latest");
    assert_eq!(plan.body.metadata, None);
}

#[test]
fn create_keeps_an_empty_input_file_id() {
    let plan = create_batch(
        &data(r#"{"input_file_id": "", "endpoint": "/v1/ocr"}"#),
        "m",
        Some("https://proxy.example/v1/"),
        &settings(None, None),
    )
    .expect("a plan");
    assert_eq!(plan.body.input_files, vec![String::new()]);
    assert_eq!(plan.body.endpoint, BatchEndpoint::Ocr);
    assert_eq!(plan.url, "https://proxy.example/v1/batch/jobs");
}

#[rstest]
#[case(r#"{"endpoint": "/v1/embeddings"}"#)]
#[case(r#"{"input_file_id": "file-1"}"#)]
fn create_rejects_a_missing_input_file_or_endpoint(#[case] request: &str) {
    let error = create_batch(&data(request), "m", None, &settings(None, None))
        .expect_err("a missing required field");
    assert_eq!(error.status_code(), None);
}

#[test]
fn create_includes_only_a_nonempty_metadata_map() {
    let empty = create_batch(
        &data(r#"{"input_file_id": "f", "endpoint": "/v1/ocr", "metadata": {}}"#),
        "m",
        None,
        &settings(None, None),
    )
    .expect("a plan");
    assert_eq!(empty.body.metadata, None);
    let named = create_batch(
        &data(r#"{"input_file_id": "f", "endpoint": "/v1/ocr", "metadata": {"name": "team"}}"#),
        "m",
        None,
        &settings(None, None),
    )
    .expect("a plan");
    assert_eq!(
        named
            .body
            .metadata
            .and_then(|metadata| metadata.get("name").cloned()),
        Some(String::from("team"))
    );
}

#[test]
fn retrieve_resolves_its_own_get_credentials() {
    let key = SecretString::from("Key");
    let request =
        retrieve_batch("job-1", None, Some(&key), &settings(None, None)).expect("a GET plan");
    assert_eq!(
        request,
        MistralGetRequest {
            method: Method::GET,
            url: String::from("https://api.mistral.ai/v1/batch/jobs/job-1"),
            headers: [(String::from("Authorization"), String::from("Bearer Key"))]
                .into_iter()
                .collect(),
        }
    );
}

#[test]
fn retrieve_falls_back_to_the_environment_key() {
    let request = retrieve_batch(
        "job-2",
        Some("https://proxy.example/v1/"),
        None,
        &settings(Some("https://env.example"), Some("EnvKey")),
    )
    .expect("a GET plan");
    assert_eq!(request.url, "https://proxy.example/v1/batch/jobs/job-2");
    assert_eq!(
        request.headers.get("Authorization").map(String::as_str),
        Some("Bearer EnvKey")
    );
}

#[test]
fn retrieve_encodes_the_batch_id_as_one_segment() {
    let key = SecretString::from("Key");
    let request =
        retrieve_batch("a b/c", None, Some(&key), &settings(None, None)).expect("a GET plan");
    assert_eq!(
        request.url,
        "https://api.mistral.ai/v1/batch/jobs/a%20b%2Fc"
    );
}

#[test]
fn retrieve_without_a_key_fails() {
    let error = retrieve_batch("job-1", None, None, &settings(None, None)).expect_err("no key");
    assert_eq!(error.status_code(), None);
    assert!(matches!(error, MistralBatchesError::Identity(_)));
}

#[rstest]
#[case("..")]
#[case("")]
fn retrieve_rejects_a_dot_or_empty_id(#[case] batch_id: &str) {
    let error = retrieve_batch(batch_id, None, None, &settings(None, Some("Key")))
        .expect_err("no GET plan");
    assert_eq!(error.status_code(), None);
}

#[test]
fn the_job_record_uses_terminal_times_only_for_the_matching_status() {
    let success = to_batch_object(&job(
        r#"{"id": "j1", "endpoint": "/v1/chat/completions", "status": "SUCCESS", "created_at": 1, "completed_at": 9, "input_files": ["file-1"], "succeeded_requests": 2}"#,
    ));
    assert_eq!(success.status, BatchStatus::Completed);
    assert_eq!(success.completed_at, Some(9));
    assert_eq!(success.failed_at, None);
    assert_eq!(success.expired_at, None);
    assert_eq!(success.cancelled_at, None);
    assert_eq!(success.input_file_id, "file-1");
    assert_eq!(success.completion_window, "24h");
    assert_eq!(success.model, None);
}

#[test]
fn the_job_record_maps_every_terminal_time() {
    let failed = to_batch_object(&job(
        r#"{"id": "j", "endpoint": "/v1/ocr", "status": "FAILED", "created_at": 1, "completed_at": 9}"#,
    ));
    assert_eq!(failed.failed_at, Some(9));
    assert_eq!(failed.completed_at, None);
    let expired = to_batch_object(&job(
        r#"{"id": "j", "endpoint": "/v1/ocr", "status": "TIMEOUT_EXCEEDED", "created_at": 1, "completed_at": 9}"#,
    ));
    assert_eq!(expired.expired_at, Some(9));
    let cancelled = to_batch_object(&job(
        r#"{"id": "j", "endpoint": "/v1/ocr", "status": "CANCELLED", "created_at": 1, "completed_at": 9}"#,
    ));
    assert_eq!(cancelled.cancelled_at, Some(9));
    let cancelling = to_batch_object(&job(
        r#"{"id": "j", "endpoint": "/v1/ocr", "status": "CANCELLATION_REQUESTED", "created_at": 1, "completed_at": 9}"#,
    ));
    assert_eq!(cancelling.status, BatchStatus::Cancelling);
    assert_eq!(cancelling.cancelled_at, None);
    assert_eq!(cancelling.completed_at, None);
}

#[test]
fn the_job_record_counts_succeeded_requests_not_completed_requests() {
    let record = to_batch_object(&job(
        r#"{"id": "j", "endpoint": "/v1/ocr", "status": "RUNNING", "created_at": 1, "total_requests": 9, "completed_requests": 99, "succeeded_requests": 3, "failed_requests": 1, "input_files": []}"#,
    ));
    let counts = record.request_counts.expect("counts");
    assert_eq!(counts.completed, 3);
    assert_eq!(counts.failed, 1);
    assert_eq!(counts.total, 9);
    assert_eq!(record.input_file_id, "");
}

#[test]
fn the_job_record_appends_the_error_count_only_above_one() {
    let record = to_batch_object(&job(
        r#"{"id": "j", "endpoint": "/v1/ocr", "status": "FAILED", "created_at": 1, "output_file": "out-1", "error_file": "err-1", "errors": [{"message": "bad"}, {"message": "many", "count": 3}, {"message": "zero", "count": 0}, {"message": "two", "count": 2}]}"#,
    ));
    let errors = record.errors.expect("errors");
    let messages: Vec<Option<&str>> = errors
        .data
        .expect("rows")
        .iter()
        .map(|row| row.message.as_deref())
        .collect();
    assert_eq!(
        messages,
        vec![
            Some("bad"),
            Some("many (x3)"),
            Some("zero"),
            Some("two (x2)")
        ]
    );
    assert_eq!(errors.object.as_deref(), Some("list"));
    assert_eq!(record.output_file_id.as_deref(), Some("out-1"));
    assert_eq!(record.error_file_id.as_deref(), Some("err-1"));
}

#[test]
fn an_empty_error_list_stays_absent() {
    let record = to_batch_object(&job(
        r#"{"id": "j", "endpoint": "/v1/ocr", "status": "QUEUED", "created_at": 1, "errors": []}"#,
    ));
    assert_eq!(record.errors, None);
}

#[test]
fn the_job_record_keeps_metadata_and_the_start_time() {
    let record: BatchObject = to_batch_object(&job(
        r#"{"id": "j", "endpoint": "/v1/ocr", "status": "RUNNING", "created_at": 1, "started_at": 5, "metadata": {"team": "a"}}"#,
    ));
    assert_eq!(record.in_progress_at, Some(5));
    assert_eq!(
        record
            .metadata
            .and_then(|metadata| metadata.get("team").cloned()),
        Some(String::from("a"))
    );
}

#[test]
fn a_job_error_without_a_count_counts_once() {
    let error: MistralBatchError = serde_json::from_str(r#"{"message": "bad"}"#).unwrap();
    assert_eq!(error.count, 1);
}

#[test]
fn an_api_failure_keeps_its_status_and_other_failures_have_none() {
    let api = MistralBatchesError::Identity(MistralIdentityError::Api {
        status: 401,
        message: String::from("denied"),
        headers: BTreeMap::new(),
    });
    assert_eq!(api.status_code(), Some(401));
    assert_eq!(MistralBatchesError::InputFileRequired.status_code(), None);
}
