use std::collections::BTreeMap;

use betterlite_openai::batches::records::BatchStatus;

use crate::batch_call::error::BatchCallError;

use super::super::fixture::{JOB_ARN, JOB_ID, REGION, lookup_job};

const INPUT_URI: &str = "s3://bucket/in/file.jsonl";
const OUTPUT_URI: &str = "s3://bucket/out";
const PREDICTED: &str = "s3://bucket/out/job-1/file.jsonl.out";

pub(super) fn completed_output_prediction() {
    completed_predicts();
    other_statuses_withhold();
    foreign_prefix();
    without_output_config();
    without_job_arn();
    metadata_defaults();
    falsey_message();
    wrong_types();
}

fn job_body(status: &str, input_uri: &str, output_uri: &str) -> String {
    format!(
        r#"{{"jobArn":"{JOB_ARN}","status":"{status}","inputDataConfig":{{"s3InputDataConfig":{{"s3Uri":"{input_uri}"}}}},"outputDataConfig":{{"s3OutputDataConfig":{{"s3Uri":"{output_uri}"}}}}}}"#
    )
}

fn completed_predicts() {
    let run = lookup_job(
        &job_body("Completed", INPUT_URI, OUTPUT_URI),
        JOB_ARN,
        Some(REGION),
        &BTreeMap::new(),
    );
    let batch = run.result.expect("the completed job projects");
    assert_eq!(batch.output_file_id.as_deref(), Some(PREDICTED));
    assert_eq!(batch.input_file_id, INPUT_URI);
    let metadata = batch.metadata.expect("the job carries metadata");
    assert_eq!(
        metadata.get("output_file_uri").map(String::as_str),
        Some(PREDICTED)
    );
    assert_eq!(
        metadata.get("output_s3_uri").map(String::as_str),
        Some(OUTPUT_URI)
    );
    assert_eq!(
        metadata.get("input_s3_uri").map(String::as_str),
        Some(INPUT_URI)
    );
    assert_eq!(metadata.get("job_arn").map(String::as_str), Some(JOB_ARN));
    assert_eq!(run.factory.requests().len(), 1);
}

fn other_statuses_withhold() {
    for status in ["InProgress", "Failed", "Cancelling", "Stopping"] {
        let run = lookup_job(
            &job_body(status, INPUT_URI, OUTPUT_URI),
            JOB_ARN,
            Some(REGION),
            &BTreeMap::new(),
        );
        let batch = run.result.expect("the job projects");
        assert_eq!(batch.output_file_id, None);
        let metadata = batch.metadata.expect("the job carries metadata");
        assert_eq!(
            metadata.get("output_file_uri").map(String::as_str),
            Some(PREDICTED)
        );
    }
}

fn foreign_prefix() {
    let run = lookup_job(
        &job_body("Completed", INPUT_URI, "s3://other-bucket/results"),
        JOB_ARN,
        Some(REGION),
        &BTreeMap::new(),
    );
    let batch = run.result.expect("the job projects");
    assert_eq!(
        batch.output_file_id.as_deref(),
        Some("s3://other-bucket/results/job-1/file.jsonl.out")
    );
}

fn without_output_config() {
    let body = format!(
        r#"{{"jobArn":"{JOB_ARN}","status":"Completed","inputDataConfig":{{"s3InputDataConfig":{{"s3Uri":"{INPUT_URI}"}}}}}}"#
    );
    let run = lookup_job(&body, JOB_ARN, Some(REGION), &BTreeMap::new());
    let batch = run.result.expect("the job projects");
    assert_eq!(batch.status, BatchStatus::Completed);
    assert_eq!(batch.output_file_id, None);
    let metadata = batch.metadata.expect("the job carries metadata");
    assert_eq!(metadata.get("output_s3_uri").map(String::as_str), Some(""));
    assert_eq!(
        metadata.get("output_file_uri").map(String::as_str),
        Some("")
    );
}

fn without_job_arn() {
    let body = format!(
        r#"{{"status":"Completed","inputDataConfig":{{"s3InputDataConfig":{{"s3Uri":"{INPUT_URI}"}}}},"outputDataConfig":{{"s3OutputDataConfig":{{"s3Uri":"{OUTPUT_URI}"}}}}}}"#
    );
    let run = lookup_job(&body, JOB_ID, Some(REGION), &BTreeMap::new());
    let batch = run.result.expect("the job projects");
    assert_eq!(batch.id, JOB_ID);
    assert_eq!(batch.output_file_id, None);
    let metadata = batch.metadata.expect("the job carries metadata");
    assert_eq!(metadata.get("job_arn").map(String::as_str), Some(JOB_ID));
}

fn metadata_defaults() {
    let run = lookup_job(
        r#"{"status":"InProgress"}"#,
        JOB_ID,
        Some(REGION),
        &BTreeMap::new(),
    );
    let batch = run.result.expect("the job projects");
    let metadata = batch.metadata.expect("the job carries metadata");
    assert_eq!(metadata.len(), 7);
    assert_eq!(metadata.get("model_arn").map(String::as_str), Some(""));
    assert_eq!(metadata.get("job_name").map(String::as_str), Some(""));
    assert_eq!(
        metadata.get("failure_message").map(String::as_str),
        Some("")
    );
}

fn falsey_message() {
    let cases = [
        (r#""message":"boom""#, "boom"),
        (r#""message":0"#, ""),
        (r#""message":0.0"#, ""),
        (r#""message":-0.0"#, ""),
        (r#""message":false"#, ""),
        (r#""message":null"#, ""),
        (r#""message":[]"#, ""),
        (r#""message":{}"#, ""),
    ];
    for (message, expected) in cases {
        let body = format!(r#"{{"jobArn":"{JOB_ARN}","status":"Failed",{message}}}"#);
        let run = lookup_job(&body, JOB_ARN, Some(REGION), &BTreeMap::new());
        let batch = run.result.expect("the job projects");
        assert_eq!(batch.failed_at, None);
        let metadata = batch.metadata.expect("the job carries metadata");
        assert_eq!(
            metadata.get("failure_message").map(String::as_str),
            Some(expected)
        );
    }
}

fn wrong_types() {
    let cases = [
        (r#"{"inputDataConfig":[]}"#, "inputDataConfig"),
        (
            r#"{"inputDataConfig":{"s3InputDataConfig":5}}"#,
            "s3InputDataConfig",
        ),
        (
            r#"{"inputDataConfig":{"s3InputDataConfig":{"s3Uri":5}}}"#,
            "s3Uri",
        ),
        (r#"{"outputDataConfig":"x"}"#, "outputDataConfig"),
        (r#"{"jobArn":5}"#, "jobArn"),
        (r#"{"modelId":5}"#, "modelId"),
        (r#"{"jobName":false}"#, "jobName"),
        (r#"{"message":1.5}"#, "message"),
        (r#"{"message":true}"#, "message"),
        (r#"{"message":[1]}"#, "message"),
        (r#"{"message":{"a":1}}"#, "message"),
    ];
    for (body, field) in cases {
        let run = lookup_job(body, JOB_ARN, Some(REGION), &BTreeMap::new());
        let error = run.result.expect_err("a wrongly typed member is an error");
        assert!(matches!(
            error,
            BatchCallError::Field { field: actual } if actual == field
        ));
    }
}
