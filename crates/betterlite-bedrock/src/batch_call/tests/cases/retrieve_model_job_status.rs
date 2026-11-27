use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::BatchStatus;

use crate::batch_call::error::BatchCallError;

use super::super::fixture::{
    ENCODED_JOB_ARN, JOB_ARN, JOB_ID, REGION, answer, auth_parameters, lookup_job, parameters,
    run_job,
};

const STATUSES: [(&str, BatchStatus); 12] = [
    ("Submitted", BatchStatus::Validating),
    ("Validating", BatchStatus::Validating),
    ("Scheduled", BatchStatus::Validating),
    ("InProgress", BatchStatus::InProgress),
    ("Stopping", BatchStatus::Cancelling),
    ("Stopped", BatchStatus::Cancelled),
    ("Completed", BatchStatus::Completed),
    ("PartiallyCompleted", BatchStatus::Completed),
    ("Failed", BatchStatus::Failed),
    ("Expired", BatchStatus::Expired),
    ("Queued", BatchStatus::InProgress),
    ("", BatchStatus::InProgress),
];

pub(super) fn retrieve_model_job_status() {
    for (name, expected) in STATUSES {
        assert_mapping(name, expected);
    }
    assert_missing_status();
    assert_body_arn_names_the_id();
    assert_region_from_arn();
    assert_explicit_region_beats_the_arn();
    blank_body();
    rejected_bodies();
    status_below_three_hundred_is_a_success();
    invalid_region_and_auth_parameter_are_refused();
}

fn status_body(status: &str) -> String {
    format!(r#"{{"jobArn":"{JOB_ARN}","status":"{status}"}}"#)
}

fn assert_mapping(name: &str, expected: BatchStatus) {
    let run = lookup_job(
        &status_body(name),
        JOB_ARN,
        Some(REGION),
        &auth_parameters(),
    );
    let batch = run.result.expect("the job status projects");
    assert_eq!(batch.status, expected);
    assert_eq!(batch.id, JOB_ARN);
    assert_eq!(batch.endpoint, "/v1/chat/completions");
    assert_eq!(batch.completion_window, "24h");
    let requests = run.factory.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].url,
        format!("https://bedrock.{REGION}.amazonaws.com/model-invocation-job/{ENCODED_JOB_ARN}")
    );
    assert!(requests[0].body.is_empty());
}

fn assert_missing_status() {
    let body = format!(r#"{{"jobArn":"{JOB_ARN}"}}"#);
    let run = lookup_job(&body, JOB_ARN, Some(REGION), &BTreeMap::new());
    let batch = run.result.expect("a missing status reads as in progress");
    assert_eq!(batch.status, BatchStatus::InProgress);
}

fn assert_body_arn_names_the_id() {
    let run = lookup_job(
        &status_body("InProgress"),
        JOB_ID,
        Some(REGION),
        &BTreeMap::new(),
    );
    let batch = run.result.expect("the job status projects");
    assert_eq!(batch.id, JOB_ARN);
    assert_eq!(batch.created_at, 0);
}

fn assert_region_from_arn() {
    let run = lookup_job(
        &status_body("InProgress"),
        JOB_ARN,
        None,
        &auth_parameters(),
    );
    assert_eq!(
        run.factory.requests()[0].url,
        format!("https://bedrock.{REGION}.amazonaws.com/model-invocation-job/{ENCODED_JOB_ARN}")
    );
    assert_eq!(run.source.regions(), [REGION]);
}

fn assert_explicit_region_beats_the_arn() {
    let run = lookup_job(
        &status_body("InProgress"),
        JOB_ARN,
        Some("ap-south-1"),
        &BTreeMap::new(),
    );
    let requests = run.factory.requests();
    assert_eq!(
        requests[0].url,
        format!("https://bedrock.ap-south-1.amazonaws.com/model-invocation-job/{ENCODED_JOB_ARN}")
    );
    assert_eq!(run.source.regions(), ["ap-south-1"]);
    let configs = run.factory.configs();
    assert_eq!(configs[0].region, "ap-south-1");
    assert_eq!(configs[0].service_name, "bedrock");
    assert_eq!(configs[0].signature_version, "v4");
}

fn blank_body() {
    let run = lookup_job("   ", JOB_ARN, Some(REGION), &BTreeMap::new());
    let batch = run.result.expect("a blank body parses as an empty record");
    assert_eq!(batch.id, JOB_ARN);
    assert_eq!(batch.status, BatchStatus::InProgress);
}

fn rejected_bodies() {
    let null = lookup_job("null", JOB_ARN, Some(REGION), &BTreeMap::new());
    assert!(matches!(
        null.result.expect_err("a null body is not a record"),
        BatchCallError::ResponseShape
    ));
    let garbage = lookup_job("not json", JOB_ARN, Some(REGION), &BTreeMap::new());
    assert!(matches!(
        garbage.result.expect_err("an unparsable body is an error"),
        BatchCallError::Decode { .. }
    ));
}

fn status_below_three_hundred_is_a_success() {
    let run = run_job(
        answer(299, &status_body("Completed")),
        JOB_ARN,
        Some(REGION),
        &BTreeMap::new(),
    );
    let batch = run.result.expect("a 299 reply parses as a job record");
    assert_eq!(batch.status, BatchStatus::Completed);
}

fn invalid_region_and_auth_parameter_are_refused() {
    for region in ["Not A Region", "", "EU"] {
        let bad_region = lookup_job(
            &status_body("InProgress"),
            JOB_ARN,
            Some(region),
            &BTreeMap::new(),
        );
        let error = bad_region
            .result
            .expect_err("a region outside AWS's pattern is refused");
        assert!(matches!(error, BatchCallError::Region(_)));
        assert!(bad_region.factory.requests().is_empty());
    }
    let arn = "arn:aws:bedrock:EU:123456789012:model-invocation-job/job-1";
    let arn_region = lookup_job(&status_body("InProgress"), arn, None, &BTreeMap::new());
    assert!(matches!(
        arn_region
            .result
            .expect_err("a region from an ARN is checked too"),
        BatchCallError::Region(_)
    ));
    let kwargs = parameters(&[("aws_access_key_id", Json::Bool(true))]);
    let bad_auth = lookup_job(&status_body("InProgress"), JOB_ARN, Some(REGION), &kwargs);
    assert!(matches!(
        bad_auth
            .result
            .expect_err("a non-string credential field is refused"),
        BatchCallError::Auth(_)
    ));
    assert!(bad_auth.factory.requests().is_empty());
}
