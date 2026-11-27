use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::{BatchObject, BatchStatus, RequestCounts};
use serde_json::value::RawValue;

use crate::batch_call::async_status::AsyncStatusCall;
use crate::batch_call::error::BatchCallError;

use super::super::async_fixture::{AsyncFactory, retrieve};
use super::super::fixture::{
    ACCESS_KEY, SECRET_KEY, SESSION_TOKEN, SUBMITTED_EPOCH, Source, credentials, now, parameter,
};

const ASYNC_ARN: &str = "arn:aws:bedrock:us-west-2:123456789012:async-invoke/job-1";
const ENCODED_ASYNC_ARN: &str =
    "arn%3Aaws%3Abedrock%3Aus-west-2%3A123456789012%3Aasync-invoke%2Fjob-1";
const MODEL_ARN: &str = "arn:aws:bedrock:us-west-2::foundation-model/model";
const REGION: &str = "us-west-2";
const MODIFIED_EPOCH: i64 = 1_705_314_700;
const END_EPOCH: i64 = 1_705_314_800;
const OUTPUT_URI: &str = "s3://out/result.jsonl";
const COMPLETED: &str = r#"{"invocationArn":"arn:aws:bedrock:us-west-2:123456789012:async-invoke/job-1","modelArn":"arn:aws:bedrock:us-west-2::foundation-model/model","status":"completed","submitTime":1705314600,"lastModifiedTime":1705314700,"endTime":1705314800,"outputDataConfig":{"s3OutputDataConfig":{"s3Uri":"s3://out/result.jsonl"}}}"#;
const FAILED: &str = r#"{"invocationArn":"arn:aws:bedrock:us-west-2:123456789012:async-invoke/job-1","modelArn":"arn:aws:bedrock:us-west-2::foundation-model/model","status":"failed","submitTime":1705314600,"lastModifiedTime":1705314700,"endTime":1705314800,"failureMessage":"boom","outputDataConfig":{"s3OutputDataConfig":{"s3Uri":"s3://out/result.jsonl"}}}"#;
const IN_PROGRESS_WITH_END: &str = r#"{"invocationArn":"arn:aws:bedrock:us-west-2:123456789012:async-invoke/job-1","modelArn":"arn:aws:bedrock:us-west-2::foundation-model/model","status":"in_progress","submitTime":1705314600,"lastModifiedTime":1705314700,"endTime":1705314800,"outputDataConfig":{"s3OutputDataConfig":{"s3Uri":"s3://out/result.jsonl"}}}"#;
const UPPERCASE: &str = r#"{"invocationArn":"arn:aws:bedrock:us-west-2:123456789012:async-invoke/job-1","modelArn":"arn:aws:bedrock:us-west-2::foundation-model/model","status":"COMPLETED","submitTime":1705314600,"lastModifiedTime":1705314700,"outputDataConfig":{"s3OutputDataConfig":{"s3Uri":"s3://out/result.jsonl"}}}"#;
const NO_CONFIG: &str = r#"{"invocationArn":"arn:aws:bedrock:us-west-2:123456789012:async-invoke/job-1","modelArn":"arn:aws:bedrock:us-west-2::foundation-model/model","status":"completed","submitTime":1705314600,"lastModifiedTime":1705314700}"#;
const NO_LAST_MODIFIED: &str = r#"{"invocationArn":"arn:aws:bedrock:us-west-2:123456789012:async-invoke/job-1","modelArn":"arn:aws:bedrock:us-west-2::foundation-model/model","status":"completed","submitTime":1705314600,"outputDataConfig":{"s3OutputDataConfig":{"s3Uri":"s3://out/result.jsonl"}}}"#;
const NO_SUBMIT_TIME: &str = r#"{"invocationArn":"arn:aws:bedrock:us-west-2:123456789012:async-invoke/job-1","modelArn":"arn:aws:bedrock:us-west-2::foundation-model/model","status":"completed","lastModifiedTime":1705314700,"outputDataConfig":{"s3OutputDataConfig":{"s3Uri":"s3://out/result.jsonl"}}}"#;
const NO_END: &str = r#"{"invocationArn":"arn:aws:bedrock:us-west-2:123456789012:async-invoke/job-1","modelArn":"arn:aws:bedrock:us-west-2::foundation-model/model","status":"completed","submitTime":1705314600,"lastModifiedTime":1705314700,"outputDataConfig":{"s3OutputDataConfig":{"s3Uri":"s3://out/result.jsonl"}}}"#;
const NO_INVOCATION_ARN: &str = r#"{"modelArn":"arn:aws:bedrock:us-west-2::foundation-model/model","status":"completed","submitTime":1705314600,"lastModifiedTime":1705314700,"outputDataConfig":{"s3OutputDataConfig":{"s3Uri":"s3://out/result.jsonl"}}}"#;
const NO_MODEL_ARN: &str = r#"{"invocationArn":"arn:aws:bedrock:us-west-2:123456789012:async-invoke/job-1","status":"completed","submitTime":1705314600,"lastModifiedTime":1705314700,"outputDataConfig":{"s3OutputDataConfig":{"s3Uri":"s3://out/result.jsonl"}}}"#;
const NO_STATUS: &str = r#"{"invocationArn":"arn:aws:bedrock:us-west-2:123456789012:async-invoke/job-1","modelArn":"arn:aws:bedrock:us-west-2::foundation-model/model","submitTime":1705314600,"lastModifiedTime":1705314700,"outputDataConfig":{"s3OutputDataConfig":{"s3Uri":"s3://out/result.jsonl"}}}"#;
const NO_S3_URI: &str = r#"{"invocationArn":"arn:aws:bedrock:us-west-2:123456789012:async-invoke/job-1","modelArn":"arn:aws:bedrock:us-west-2::foundation-model/model","status":"completed","submitTime":1705314600,"lastModifiedTime":1705314700,"outputDataConfig":{"s3OutputDataConfig":{}}}"#;

pub(super) fn retrieve_async_status() {
    completed();
    failed();
    in_progress_with_end_time();
    invalid_status();
    missing_fields();
    wrong_types();
    falsey_failure_messages();
    null_end_time();
    null_last_modified_time();
    status_words();
    no_end_time();
    http_failure();
}

fn run(status: u16, body: &str) -> (Result<BatchObject, BatchCallError>, AsyncFactory) {
    let source = Source::new(credentials());
    let factory = AsyncFactory::new(status, body);
    let mut params = async_parameters();
    let environment = BTreeMap::new();
    let call = AsyncStatusCall {
        invocation_arn: ASYNC_ARN,
        region: REGION,
        optional_params: &mut params,
        environment: &environment,
        now: now(),
    };
    let result = retrieve(call, &source, &factory);
    (result, factory)
}

fn async_parameters() -> BTreeMap<String, Box<RawValue>> {
    BTreeMap::from([
        parameter("aws_access_key_id", &Json::Text(ACCESS_KEY.to_owned())),
        parameter("aws_secret_access_key", &Json::Text(SECRET_KEY.to_owned())),
        parameter("aws_session_token", &Json::Text(SESSION_TOKEN.to_owned())),
    ])
}

fn body_with(extra: &str) -> String {
    format!(
        r#"{{"invocationArn":"{ASYNC_ARN}","modelArn":"{MODEL_ARN}","status":"completed","submitTime":1705314600,"lastModifiedTime":1705314700,"outputDataConfig":{{"s3OutputDataConfig":{{"s3Uri":"{OUTPUT_URI}"}}}},{extra}}}"#
    )
}

fn completed() {
    let (result, factory) = run(200, COMPLETED);
    let batch = result.expect("the async status projects");
    assert_eq!(batch.id, ASYNC_ARN);
    assert_eq!(batch.status, BatchStatus::Completed);
    assert_eq!(batch.created_at, SUBMITTED_EPOCH);
    assert_eq!(batch.in_progress_at, Some(MODIFIED_EPOCH));
    assert_eq!(batch.completed_at, Some(END_EPOCH));
    assert_eq!(batch.failed_at, None);
    assert_eq!(
        batch.request_counts,
        Some(RequestCounts {
            completed: 1,
            failed: 0,
            total: 1
        })
    );
    assert_eq!(batch.endpoint, "/v1/embeddings");
    assert_eq!(batch.completion_window, "24h");
    assert_eq!(batch.input_file_id, "");
    assert_eq!(batch.output_file_id, None);
    assert_metadata(&batch, OUTPUT_URI, "");
    let requests = factory.requests();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .url
            .ends_with(&format!("/async-invoke/{ENCODED_ASYNC_ARN}"))
    );
}

fn failed() {
    let (result, _factory) = run(200, FAILED);
    let batch = result.expect("the async status projects");
    assert_eq!(batch.status, BatchStatus::Failed);
    assert_eq!(
        batch.request_counts,
        Some(RequestCounts {
            completed: 0,
            failed: 1,
            total: 1
        })
    );
    assert_eq!(batch.completed_at, Some(END_EPOCH));
    assert_eq!(batch.failed_at, Some(END_EPOCH));
    assert_metadata(&batch, OUTPUT_URI, "boom");
}

// the handler assigns endTime to completed_at whatever the status word says
fn in_progress_with_end_time() {
    let (result, _factory) = run(200, IN_PROGRESS_WITH_END);
    let batch = result.expect("the async status projects");
    assert_eq!(batch.status, BatchStatus::InProgress);
    assert_eq!(batch.completed_at, Some(END_EPOCH));
    assert_eq!(batch.failed_at, None);
    assert_eq!(
        batch.request_counts,
        Some(RequestCounts {
            completed: 0,
            failed: 0,
            total: 1
        })
    );
}

fn invalid_status() {
    let (result, _factory) = run(200, UPPERCASE);
    let error = result.expect_err("the raw status must name an OpenAI batch status");
    assert!(matches!(error, BatchCallError::InvalidStatus { status } if status == "COMPLETED"));
}

fn missing_fields() {
    let missing = [
        (NO_CONFIG, "outputDataConfig"),
        (NO_LAST_MODIFIED, "lastModifiedTime"),
        (NO_SUBMIT_TIME, "submitTime"),
        (NO_INVOCATION_ARN, "invocationArn"),
        (NO_MODEL_ARN, "modelArn"),
        (NO_STATUS, "status"),
        (NO_S3_URI, "s3Uri"),
    ];
    for (body, field) in missing {
        let (result, _factory) = run(200, body);
        let error = result.expect_err("a required member is missing");
        assert!(matches!(
            error,
            BatchCallError::MissingField { field: actual } if actual == field
        ));
    }
}

fn wrong_types() {
    let cases = [
        (r#""invocationArn":5"#, "invocationArn"),
        (r#""modelArn":[]"#, "modelArn"),
        (r#""submitTime":"x""#, "submitTime"),
        (r#""submitTime":true"#, "submitTime"),
        (r#""lastModifiedTime":"x""#, "lastModifiedTime"),
        (r#""endTime":"x""#, "endTime"),
        (r#""failureMessage":true"#, "failureMessage"),
        (r#""outputDataConfig":[]"#, "outputDataConfig"),
        (
            r#""outputDataConfig":{"s3OutputDataConfig":5}"#,
            "s3OutputDataConfig",
        ),
        (
            r#""outputDataConfig":{"s3OutputDataConfig":{"s3Uri":5}}"#,
            "s3Uri",
        ),
    ];
    for (extra, field) in cases {
        let (result, _factory) = run(200, &body_with(extra));
        let error = result.expect_err("a wrongly typed member is an error");
        assert!(matches!(error, BatchCallError::Field { field: actual } if actual == field));
    }
}

fn falsey_failure_messages() {
    let cases = [
        (r#""failureMessage":0"#, ""),
        (r#""failureMessage":0.0"#, ""),
        (r#""failureMessage":[]"#, ""),
        (r#""failureMessage":{}"#, ""),
        (r#""failureMessage":"boom""#, "boom"),
    ];
    for (extra, expected) in cases {
        let (result, _factory) = run(200, &body_with(extra));
        let batch = result.expect("the async status projects");
        assert_metadata(&batch, OUTPUT_URI, expected);
    }
}

fn null_end_time() {
    let (result, _factory) = run(200, &body_with(r#""endTime":null"#));
    let batch = result.expect("the async status projects");
    assert_eq!(batch.completed_at, None);
}

fn null_last_modified_time() {
    let (result, _factory) = run(200, &body_with(r#""lastModifiedTime":null"#));
    let batch = result.expect("the async status projects");
    assert_eq!(batch.in_progress_at, None);
}

fn status_words() {
    let words = [
        "validating",
        "failed",
        "in_progress",
        "finalizing",
        "completed",
        "expired",
        "cancelling",
        "cancelled",
    ];
    let expected = [
        BatchStatus::Validating,
        BatchStatus::Failed,
        BatchStatus::InProgress,
        BatchStatus::Finalizing,
        BatchStatus::Completed,
        BatchStatus::Expired,
        BatchStatus::Cancelling,
        BatchStatus::Cancelled,
    ];
    for (word, status) in words.into_iter().zip(expected) {
        let body = format!(
            r#"{{"invocationArn":"{ASYNC_ARN}","modelArn":"{MODEL_ARN}","status":"{word}","submitTime":1705314600,"lastModifiedTime":1705314700,"outputDataConfig":{{"s3OutputDataConfig":{{"s3Uri":"{OUTPUT_URI}"}}}}}}"#
        );
        let (result, _factory) = run(200, &body);
        let batch = result.expect("the async status projects");
        assert_eq!(batch.status, status);
    }
}

fn no_end_time() {
    let (result, _factory) = run(200, NO_END);
    let batch = result.expect("the async status projects");
    assert_eq!(batch.status, BatchStatus::Completed);
    assert_eq!(batch.completed_at, None);
    assert_eq!(batch.failed_at, None);
}

fn http_failure() {
    let (result, _factory) = run(500, r#"{"code":"InternalServerException"}"#);
    let error = result.expect_err("a failed reply is an error");
    assert!(matches!(error, BatchCallError::Async(_)));
}

fn assert_metadata(batch: &BatchObject, output: &str, failure: &str) {
    let metadata = batch
        .metadata
        .as_ref()
        .expect("the async status carries metadata");
    assert_eq!(
        metadata.get("output_file_id").map(String::as_str),
        Some(output)
    );
    assert_eq!(
        metadata.get("failure_message").map(String::as_str),
        Some(failure)
    );
    assert_eq!(
        metadata.get("model_arn").map(String::as_str),
        Some(MODEL_ARN)
    );
}
