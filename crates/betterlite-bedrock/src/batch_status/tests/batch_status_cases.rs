#![cfg(test)]

use std::collections::BTreeMap;
use std::future::ready;

use betterlite_openai::batches::records::{BatchObjectKind, BatchStatus};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rstest::rstest;

use crate::aws_settings::sources::Credentials;
use crate::batch_status::error::BatchStatusError;
use crate::batch_status::error_class::error_class;
use crate::batch_status::handler::{
    EpochInput, SIGNATURE_VERSION, extract_job_id_from_arn, predict_output_file_uri,
    record_counts_from_response, region_from_bedrock_arn, to_epoch,
};
use crate::batch_status::request::{RetrieveRequest, retrieve_signed_request};
use crate::batch_status::tests::support::{
    JOB_ARN, NOW, SECRET, counts, credentials, decode, decode_with, json, metadata_value, params,
    response, runtime, signing_now,
};
use crate::bedrock_settings::{ErrorHeaders, HeaderValue};
use crate::credentials::error::CredentialError;
use crate::request_policy::{Params, RequestValue};
use crate::signing::request::AwsSignedRequest;

#[rstest]
#[case::batch_status_table(batch_status_table)]
#[case::unknown_status_default(unknown_status_default)]
#[case::timestamps_and_null_fields(timestamps_and_null_fields)]
#[case::record_counts(record_counts)]
#[case::output_uri_prediction(output_uri_prediction)]
#[case::errors_and_metadata(errors_and_metadata)]
#[case::retrieve_signed_request(retrieve_signed_request_case)]
#[case::file_config_shapes(file_config_shapes)]
#[case::handler_region_and_time(handler_region_and_time)]
fn batch_status_cases(#[case] case: fn()) {
    case();
}

fn status_body(status: &str) -> String {
    format!(r#"{{"jobArn":"{JOB_ARN}","status":"{status}"}}"#)
}

fn batch_status_table() {
    status_table_map();
    fixed_projection_fields();
}

fn status_table_map() {
    let names = [
        "Submitted",
        "Validating",
        "Scheduled",
        "InProgress",
        "PartiallyCompleted",
        "Completed",
        "Failed",
        "Stopping",
        "Stopped",
        "Expired",
    ];
    let expected = [
        BatchStatus::Validating,
        BatchStatus::Validating,
        BatchStatus::InProgress,
        BatchStatus::InProgress,
        BatchStatus::Completed,
        BatchStatus::Completed,
        BatchStatus::Failed,
        BatchStatus::Cancelling,
        BatchStatus::Cancelled,
        BatchStatus::Expired,
    ];
    let actual: Vec<BatchStatus> = names
        .iter()
        .map(|name| {
            decode(&status_body(name))
                .expect("the status decodes")
                .status
        })
        .collect();
    assert_eq!(actual, expected.to_vec());
}

fn fixed_projection_fields() {
    let batch = decode(&status_body("Submitted")).expect("the status decodes");
    assert_eq!(batch.object, BatchObjectKind::Batch);
    assert_eq!(batch.endpoint, "/v1/chat/completions");
    assert_eq!(batch.completion_window, "24h");
    assert_eq!(batch.id, JOB_ARN);
    assert_eq!(batch.created_at, NOW);
    assert_eq!(batch.error_file_id, None);
    assert_eq!(batch.finalizing_at, None);
    assert_eq!(batch.expired_at, None);
    assert_eq!(batch.cancelling_at, None);
    assert_eq!(batch.request_counts, None);
    let absent_arn = decode(r#"{"status":"Submitted"}"#).expect("the status decodes");
    assert_eq!(absent_arn.id, "");
}

fn unknown_status_default() {
    unknown_status_fallbacks();
    malformed_bodies();
    malformed_records();
}

fn unknown_status_fallbacks() {
    let missing = decode(r#"{"jobArn":"a"}"#).expect("the default status decodes");
    assert_eq!(missing.status, BatchStatus::Validating);
    let null = decode(r#"{"jobArn":"a","status":null}"#).expect("the null status decodes");
    assert_eq!(null.status, BatchStatus::Validating);
    let unknown =
        decode(r#"{"jobArn":"a","status":"Queued"}"#).expect("the unknown status decodes");
    assert_eq!(unknown.status, BatchStatus::Validating);
    let modified =
        decode(r#"{"jobArn":"a","status":"Queued","lastModifiedTime":"2024-01-15T10:30:00Z"}"#)
            .expect("the unknown status decodes");
    assert_eq!(modified.in_progress_at, None);
}

fn malformed_bodies() {
    assert!(matches!(
        decode("not json"),
        Err(BatchStatusError::Parse { .. })
    ));
    assert!(matches!(
        decode(r#"{"jobArn":"a""#),
        Err(BatchStatusError::Parse { .. })
    ));
    assert!(matches!(
        decode(r#"["Submitted"]"#),
        Err(BatchStatusError::ResponseShape)
    ));
}

fn malformed_records() {
    assert!(matches!(
        decode(r#"{"jobArn":null,"status":"Submitted"}"#),
        Err(BatchStatusError::Field { field: "jobArn" })
    ));
    assert!(matches!(
        decode(r#"{"jobArn":5,"status":"Submitted"}"#),
        Err(BatchStatusError::Field { field: "jobArn" })
    ));
}

fn timestamps_and_null_fields() {
    running_timestamps();
    terminal_timestamps();
    timestamp_fallbacks();
    timestamp_fractions();
    timestamp_formats();
    timestamp_edge_forms();
}

fn running_timestamps() {
    let running = decode(
        r#"{"jobArn":"a","status":"InProgress","submitTime":"2024-01-15T10:30:00Z","lastModifiedTime":"2024-01-15T10:31:00Z","endTime":"2024-01-15T10:32:00Z","jobExpirationTime":"2024-01-16T10:30:00Z"}"#,
    )
    .expect("the running job decodes");
    assert_eq!(running.created_at, 1_705_314_600);
    assert_eq!(running.in_progress_at, Some(1_705_314_660));
    assert_eq!(running.completed_at, None);
    assert_eq!(running.failed_at, None);
    assert_eq!(running.cancelled_at, None);
    assert_eq!(running.expires_at, Some(1_705_401_000));
    assert_eq!(running.finalizing_at, None);
    assert_eq!(running.expired_at, None);
    assert_eq!(running.cancelling_at, None);
    assert_eq!(running.error_file_id, None);
    let validating =
        decode(r#"{"jobArn":"a","status":"Validating","lastModifiedTime":"2024-01-15T10:31:00Z"}"#)
            .expect("the validating job decodes");
    assert_eq!(validating.in_progress_at, Some(1_705_314_660));
    let scheduled =
        decode(r#"{"jobArn":"a","status":"Scheduled","lastModifiedTime":"2024-01-15T10:31:00Z"}"#)
            .expect("the scheduled job decodes");
    assert_eq!(scheduled.in_progress_at, Some(1_705_314_660));
}

fn terminal_timestamps() {
    let completed = decode(
        r#"{"jobArn":"a","status":"Completed","endTime":"2024-01-15T10:32:00Z","lastModifiedTime":"2024-01-15T10:31:00Z"}"#,
    )
    .expect("the completed job decodes");
    assert_eq!(completed.completed_at, Some(1_705_314_720));
    assert_eq!(completed.in_progress_at, None);
    assert_eq!(completed.failed_at, None);
    let partial =
        decode(r#"{"jobArn":"a","status":"PartiallyCompleted","endTime":"2024-01-15T10:32:00Z"}"#)
            .expect("the partial job decodes");
    assert_eq!(partial.completed_at, Some(1_705_314_720));
    let failed = decode(r#"{"jobArn":"a","status":"Failed","endTime":"2024-01-15T10:32:00Z"}"#)
        .expect("the failed job decodes");
    assert_eq!(failed.failed_at, Some(1_705_314_720));
    assert_eq!(failed.completed_at, None);
    let stopped = decode(r#"{"jobArn":"a","status":"Stopped","endTime":"2024-01-15T10:32:00Z"}"#)
        .expect("the stopped job decodes");
    assert_eq!(stopped.cancelled_at, Some(1_705_314_720));
    let stopping = decode(
        r#"{"jobArn":"a","status":"Stopping","endTime":"2024-01-15T10:32:00Z","lastModifiedTime":"2024-01-15T10:31:00Z"}"#,
    )
    .expect("the stopping job decodes");
    assert_eq!(stopping.cancelled_at, None);
    assert_eq!(stopping.completed_at, None);
    assert_eq!(stopping.failed_at, None);
    assert_eq!(stopping.in_progress_at, None);
}

fn timestamp_fallbacks() {
    let malformed = decode(
        r#"{"jobArn":"a","status":"InProgress","submitTime":"not-a-time","jobExpirationTime":"2024-13-45T00:00:00Z"}"#,
    )
    .expect("the malformed times decode");
    assert_eq!(malformed.created_at, NOW);
    assert_eq!(malformed.expires_at, None);
    let epoch_zero =
        decode(r#"{"jobArn":"a","status":"Submitted","submitTime":"1970-01-01T00:00:00Z"}"#)
            .expect("the epoch job decodes");
    assert_eq!(epoch_zero.created_at, NOW);
    let nulls = decode(
        r#"{"jobArn":"a","status":"InProgress","submitTime":null,"lastModifiedTime":null,"jobExpirationTime":null}"#,
    )
    .expect("the null times decode");
    assert_eq!(nulls.created_at, NOW);
    assert_eq!(nulls.in_progress_at, None);
    assert_eq!(nulls.expires_at, None);
    let numbers = decode(
        r#"{"jobArn":"a","status":"Submitted","submitTime":1705314600,"jobExpirationTime":1705314600}"#,
    )
    .expect("the numeric times decode");
    assert_eq!(numbers.created_at, NOW);
    assert_eq!(numbers.expires_at, None);
}

fn timestamp_fractions() {
    let fractions = decode(
        r#"{"jobArn":"a","status":"Submitted","jobExpirationTime":"1969-12-31T23:59:59.5Z"}"#,
    )
    .expect("the fraction job decodes");
    assert_eq!(fractions.expires_at, Some(0));
    let negative = decode(
        r#"{"jobArn":"a","status":"Submitted","jobExpirationTime":"1969-12-31T23:59:58.5Z"}"#,
    )
    .expect("the negative fraction decodes");
    assert_eq!(negative.expires_at, Some(-1));
    let positive = decode(
        r#"{"jobArn":"a","status":"Submitted","submitTime":"2024-01-15T10:30:00.250Z","jobExpirationTime":"1970-01-01T00:00:00.5Z"}"#,
    )
    .expect("the positive fraction decodes");
    assert_eq!(positive.created_at, 1_705_314_600);
    assert_eq!(positive.expires_at, Some(0));
    let whole =
        decode(r#"{"jobArn":"a","status":"Submitted","jobExpirationTime":"1969-12-31T23:59:58Z"}"#)
            .expect("the whole negative second decodes");
    assert_eq!(whole.expires_at, Some(-2));
}

fn timestamp_formats() {
    let naive = decode(
        r#"{"jobArn":"a","status":"Submitted","submitTime":"2024-01-15T10:30:00","jobExpirationTime":"2024-01-15 10:30:00"}"#,
    )
    .expect("the naive times decode");
    assert_eq!(naive.created_at, 1_705_314_600);
    assert_eq!(naive.expires_at, Some(1_705_314_600));
    let offset =
        decode(r#"{"jobArn":"a","status":"Submitted","submitTime":"2024-01-15T10:30:00+05:30"}"#)
            .expect("the offset time decodes");
    assert_eq!(offset.created_at, 1_705_294_800);
    let new_year =
        decode(r#"{"jobArn":"a","status":"Submitted","submitTime":"2024-01-01T00:00:00"}"#)
            .expect("the new year time decodes");
    assert_eq!(new_year.created_at, 1_704_067_200);
    let earliest =
        decode(r#"{"jobArn":"a","status":"Submitted","submitTime":"0001-01-02T00:00:00"}"#)
            .expect("the earliest accepted naive time decodes");
    assert_eq!(earliest.created_at, -62_135_510_400);
}

fn timestamp_edge_forms() {
    rejected_timestamp_forms();
    offset_timestamp_forms();
}

fn rejected_timestamp_forms() {
    let leap = decode(
        r#"{"jobArn":"a","status":"InProgress","submitTime":"2024-06-30T23:59:60Z","jobExpirationTime":"2024-06-30T23:59:60Z"}"#,
    )
    .expect("the leap second decodes");
    assert_eq!(leap.created_at, NOW);
    assert_eq!(leap.expires_at, None);
    let lower = decode(
        r#"{"jobArn":"a","status":"InProgress","submitTime":"2024-01-15T10:30:00z","jobExpirationTime":"2024-01-15T10:30:00z"}"#,
    )
    .expect("the lowercase zone decodes");
    assert_eq!(lower.created_at, NOW);
    assert_eq!(lower.expires_at, None);
    let spaced = decode(
        r#"{"jobArn":"a","status":"InProgress","submitTime":"2024-01-15T 10:30:00","jobExpirationTime":"2024-01-15T 10:30:00"}"#,
    )
    .expect("the spaced time decodes");
    assert_eq!(spaced.created_at, NOW);
    assert_eq!(spaced.expires_at, None);
    let year_zero = decode(
        r#"{"jobArn":"a","status":"InProgress","submitTime":"0000-01-01T00:00:00Z","jobExpirationTime":"0000-01-01T00:00:00Z"}"#,
    )
    .expect("the zero year decodes");
    assert_eq!(year_zero.created_at, NOW);
    assert_eq!(year_zero.expires_at, None);
    let separator = decode(
        r#"{"jobArn":"a","status":"InProgress","submitTime":"2024x01-15T00:00:00Z","jobExpirationTime":"2024x01-15T00:00:00Z"}"#,
    )
    .expect("the bad separator decodes");
    assert_eq!(separator.created_at, NOW);
    assert_eq!(separator.expires_at, None);
    let early = decode(
        r#"{"jobArn":"a","status":"InProgress","submitTime":"0001-01-01T00:00:00","jobExpirationTime":"0001-01-01T00:00:00"}"#,
    )
    .expect("the early naive time decodes");
    assert_eq!(early.created_at, NOW);
    assert_eq!(early.expires_at, None);
}

fn offset_timestamp_forms() {
    let minute = decode(
        r#"{"jobArn":"a","status":"Submitted","submitTime":"2024-01-15T10:30Z","jobExpirationTime":"2024-01-15T10:30+05:30"}"#,
    )
    .expect("the minute offsets decode");
    assert_eq!(minute.created_at, 1_705_314_600);
    assert_eq!(minute.expires_at, Some(1_705_294_800));
    let hour = decode(r#"{"jobArn":"a","status":"Submitted","submitTime":"2024-01-15T10"}"#)
        .expect("the hour-only time decodes");
    assert_eq!(hour.created_at, 1_705_312_800);
    let naive_minute =
        decode(r#"{"jobArn":"a","status":"Submitted","submitTime":"2024-01-15T10:30"}"#)
            .expect("the naive minute decodes");
    assert_eq!(naive_minute.created_at, 1_705_314_600);
    let hour_zone = decode(r#"{"jobArn":"a","status":"Submitted","submitTime":"2024-01-15T10Z"}"#)
        .expect("the hour-only zone decodes");
    assert_eq!(hour_zone.created_at, 1_705_312_800);
    let negative =
        decode(r#"{"jobArn":"a","status":"Submitted","submitTime":"2024-01-15T10:30:00-05:30"}"#)
            .expect("the negative offset decodes");
    assert_eq!(negative.created_at, 1_705_334_400);
    let hour_offset =
        decode(r#"{"jobArn":"a","status":"Submitted","submitTime":"2024-01-15T10:30:00+05"}"#)
            .expect("the hour offset decodes");
    assert_eq!(hour_offset.created_at, 1_705_296_600);
    let compact =
        decode(r#"{"jobArn":"a","status":"Submitted","submitTime":"2024-01-15T10:30:00+0530"}"#)
            .expect("the compact offset decodes");
    assert_eq!(compact.created_at, 1_705_294_800);
}

fn record_counts() {
    record_count_shapes();
    record_count_edge_values();
    record_count_mismatches();
}

fn record_count_shapes() {
    let full = response(&[
        ("totalRecordCount", "10"),
        ("successRecordCount", "7"),
        ("errorRecordCount", "3"),
    ]);
    assert_eq!(
        record_counts_from_response(&full).unwrap(),
        Some(counts(7, 3, 10))
    );
    let missing_total = response(&[("successRecordCount", "7")]);
    assert_eq!(record_counts_from_response(&missing_total).unwrap(), None);
    let missing_success = response(&[("totalRecordCount", "10")]);
    assert_eq!(record_counts_from_response(&missing_success).unwrap(), None);
    let text_total = response(&[("totalRecordCount", "\"10\""), ("successRecordCount", "7")]);
    assert_eq!(record_counts_from_response(&text_total).unwrap(), None);
    let float_success = response(&[("totalRecordCount", "10"), ("successRecordCount", "7.0")]);
    assert_eq!(record_counts_from_response(&float_success).unwrap(), None);
}

fn record_count_edge_values() {
    let flags = response(&[
        ("totalRecordCount", "true"),
        ("successRecordCount", "false"),
        ("errorRecordCount", "true"),
    ]);
    assert_eq!(
        record_counts_from_response(&flags).unwrap(),
        Some(counts(0, 1, 1))
    );
    let negatives = response(&[
        ("totalRecordCount", "-2"),
        ("successRecordCount", "-1"),
        ("errorRecordCount", "-4"),
    ]);
    assert_eq!(
        record_counts_from_response(&negatives).unwrap(),
        Some(counts(-1, -4, -2))
    );
}

fn record_count_mismatches() {
    let absent_errors = response(&[("totalRecordCount", "10"), ("successRecordCount", "7")]);
    assert_eq!(
        record_counts_from_response(&absent_errors).unwrap(),
        Some(counts(7, 0, 10))
    );
    let more_failures = response(&[
        ("totalRecordCount", "2"),
        ("successRecordCount", "0"),
        ("errorRecordCount", "5"),
    ]);
    assert_eq!(
        record_counts_from_response(&more_failures).unwrap(),
        Some(counts(0, 5, 2))
    );
    let huge = response(&[
        ("totalRecordCount", "100000000000000000000"),
        ("successRecordCount", "7"),
    ]);
    assert!(matches!(
        record_counts_from_response(&huge),
        Err(BatchStatusError::CountRange {
            field: "totalRecordCount"
        })
    ));
}

fn output_uri_prediction() {
    job_id_extraction();
    output_uri_shapes();
    output_uri_missing_ingredients();
}

fn job_id_extraction() {
    assert_eq!(extract_job_id_from_arn(JOB_ARN), Some("job-abc".to_owned()));
    assert_eq!(extract_job_id_from_arn("job-abc"), None);
    assert_eq!(
        extract_job_id_from_arn("arn:aws:bedrock:us-east-1:1:model-invocation-job/"),
        None
    );
    assert_eq!(
        extract_job_id_from_arn("x:model-invocation-job/job"),
        Some("job".to_owned())
    );
}

fn output_uri_shapes() {
    assert_eq!(
        predict_output_file_uri(
            "s3://bucket/out",
            "s3://bucket/in/file.jsonl",
            Some("job-abc")
        )
        .as_deref(),
        Some("s3://bucket/out/job-abc/file.jsonl.out")
    );
    assert_eq!(
        predict_output_file_uri(
            "s3://bucket/out/",
            "s3://bucket/in/file.jsonl",
            Some("job-abc")
        )
        .as_deref(),
        Some("s3://bucket/out/job-abc/file.jsonl.out")
    );
}

fn output_uri_missing_ingredients() {
    assert_eq!(
        predict_output_file_uri("", "s3://bucket/in/file.jsonl", Some("job-abc")),
        None
    );
    assert_eq!(
        predict_output_file_uri("s3://bucket/out", "", Some("job-abc")),
        None
    );
    assert_eq!(
        predict_output_file_uri("s3://bucket/out", "s3://bucket/in/file.jsonl", None),
        None
    );
    assert_eq!(
        predict_output_file_uri("s3://bucket/out", "s3://bucket/in/file.jsonl", Some("")),
        None
    );
    assert_eq!(
        predict_output_file_uri("s3://bucket/out", "s3://bucket/dir/", Some("job-abc")),
        None
    );
    assert_eq!(
        predict_output_file_uri(
            "s3://bucket/out",
            "s3://bucket/in/file%20name.jsonl?versionId=abc",
            Some("job-abc")
        )
        .as_deref(),
        Some("s3://bucket/out/job-abc/file%20name.jsonl?versionId=abc.out")
    );
    assert_eq!(
        predict_output_file_uri("s3://bucket/out", "/leading", Some("job-abc")).as_deref(),
        Some("s3://bucket/out/job-abc/leading.out")
    );
}

fn errors_and_metadata() {
    error_entries();
    metadata_fields();
    error_class_mapping();
}

fn error_entries() {
    let failed = decode_with(
        r#"{"jobArn":"a","message":"job failed","jobName":"name"}"#,
        500,
    )
    .expect("the failed job decodes");
    let errors = failed.errors.as_ref().expect("the message becomes errors");
    assert_eq!(errors.object.as_deref(), Some("list"));
    let entries = errors.data.as_ref().expect("the errors carry one entry");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].code.as_deref(), Some("500"));
    assert_eq!(entries[0].message.as_deref(), Some("job failed"));
    assert_eq!(entries[0].line, None);
    assert_eq!(entries[0].param, None);
    let plain = decode_with(r#"{"jobArn":"a","message":"boom"}"#, 429).expect("the job decodes");
    let plain_entries = plain
        .errors
        .as_ref()
        .and_then(|failures| failures.data.as_ref())
        .expect("the errors carry one entry");
    assert_eq!(plain_entries[0].code.as_deref(), Some("429"));
    let empty = decode(r#"{"jobArn":"a","message":""}"#).expect("the job decodes");
    assert_eq!(empty.errors, None);
    let null = decode(r#"{"jobArn":"a","message":null}"#).expect("the job decodes");
    assert_eq!(null.errors, None);
    let absent = decode(r#"{"jobArn":"a"}"#).expect("the job decodes");
    assert_eq!(absent.errors, None);
}

fn metadata_fields() {
    let failed = decode_with(
        r#"{"jobArn":"a","jobName":"name","clientRequestToken":"token","modelId":"model","roleArn":"role","timeoutDurationInHours":24,"vpcConfig":{"securityGroupIds":["sg-1"],"subnetIds":["subnet-1"]},"unusedField":"ignored"}"#,
        200,
    )
    .expect("the job decodes");
    assert_eq!(metadata_value(&failed, "jobName").as_deref(), Some("name"));
    assert_eq!(
        metadata_value(&failed, "clientRequestToken").as_deref(),
        Some("token")
    );
    assert_eq!(metadata_value(&failed, "modelId").as_deref(), Some("model"));
    assert_eq!(metadata_value(&failed, "roleArn").as_deref(), Some("role"));
    assert_eq!(
        metadata_value(&failed, "timeoutDurationInHours").as_deref(),
        Some("24")
    );
    assert_eq!(
        metadata_value(&failed, "vpcConfig").as_deref(),
        Some(r#"{"securityGroupIds": ["sg-1"], "subnetIds": ["subnet-1"]}"#)
    );
    assert_eq!(
        failed.metadata.as_ref().map(IndexMap::len),
        Some(6),
        "only the six enriched fields are kept"
    );
    let listed = decode(r#"{"jobArn":"a","vpcConfig":["a"]}"#).expect("the job decodes");
    assert_eq!(
        metadata_value(&listed, "vpcConfig").as_deref(),
        Some(r#"["a"]"#)
    );
    let flag = decode(r#"{"jobArn":"a","timeoutDurationInHours":true}"#).expect("the job decodes");
    assert_eq!(
        metadata_value(&flag, "timeoutDurationInHours").as_deref(),
        Some("True")
    );
    let omitted =
        decode(r#"{"jobArn":"a","jobName":null,"modelId":"model"}"#).expect("the job decodes");
    assert_eq!(omitted.metadata.as_ref().map(IndexMap::len), Some(1));
}

fn error_class_mapping() {
    let headers = ErrorHeaders::Http(vec![("retry-after".to_owned(), "1".to_owned())]);
    let class =
        error_class("boom".to_owned(), 503, headers.clone()).expect("the headers are ASCII");
    assert_eq!(class.error.status_code, 503);
    assert_eq!(class.error.message, "boom");
    assert_eq!(
        class.error.headers,
        Some(vec![("retry-after".to_owned(), "1".to_owned())])
    );
    assert_eq!(class.headers, Some(headers));
    let mapping = ErrorHeaders::Mapping(vec![
        ("x-text".to_owned(), HeaderValue::Text("kept".to_owned())),
        ("x-bytes".to_owned(), HeaderValue::Bytes(b"ok".to_vec())),
        ("x-count".to_owned(), HeaderValue::Integer(2)),
    ]);
    let mapped =
        error_class("bad".to_owned(), 400, mapping.clone()).expect("the headers are ASCII");
    assert_eq!(mapped.error.status_code, 400);
    assert_eq!(
        mapped.error.headers,
        Some(vec![("x-text".to_owned(), "kept".to_owned())]),
        "the provider error keeps text headers only"
    );
    assert_eq!(
        mapped.response_headers,
        vec![
            ("x-text".to_owned(), "kept".to_owned()),
            ("x-bytes".to_owned(), "ok".to_owned()),
        ]
    );
    assert_eq!(
        mapped.headers,
        Some(mapping),
        "the typed headers stay whole"
    );
}

fn retrieve_signed_request_case() {
    signed_get_request();
    signing_region_selection();
    signing_region_from_params();
    malformed_signing_params();
    arn_validation_errors();
}

fn signed_get(batch_id: &str, deployment: &Params, request_params: &Params) -> AwsSignedRequest {
    let retrieve = retrieve_signed_request(
        RetrieveRequest {
            batch_id,
            litellm_params: deployment,
            optional_params: request_params,
            now: signing_now(),
        },
        |_settings, _region| ready(Ok::<Credentials, CredentialError>(credentials())),
    );
    runtime().block_on(retrieve).expect("the request signs")
}

fn header_map(signed: &AwsSignedRequest) -> BTreeMap<String, String> {
    signed.headers.iter().cloned().collect()
}

fn signed_get_request() {
    let deployment = params(&[
        ("aws_access_key_id", "AKIDEXAMPLE"),
        ("aws_secret_access_key", SECRET),
        ("aws_region_name", "us-west-2"),
    ]);
    let signed = signed_get(JOB_ARN, &deployment, &Params::new());
    assert_eq!(signed.method, "GET");
    assert_eq!(
        signed.url,
        "https://bedrock.us-east-1.amazonaws.com/model-invocation-job/arn%3Aaws%3Abedrock%3Aus-east-1%3A123456789012%3Amodel-invocation-job%2Fjob-abc"
    );
    assert!(signed.body.is_empty());
    let headers = header_map(&signed);
    assert_eq!(
        headers.get("X-Amz-Date").map(String::as_str),
        Some("20260101T000000Z")
    );
    let authorization = headers.get("Authorization").expect("the request signs");
    assert!(
        authorization.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-west-2/bedrock/aws4_request, SignedHeaders=host;x-amz-date, Signature="
        ),
        "{authorization}"
    );
    assert_eq!(headers.len(), 2);
}

fn signing_region_selection() {
    let deployment = params(&[
        ("aws_access_key_id", "AKIDEXAMPLE"),
        ("aws_secret_access_key", SECRET),
        ("aws_region_name", "us-west-2"),
    ]);
    let request_region = params(&[("aws_region_name", "eu-west-1")]);
    let preferred = signed_get(JOB_ARN, &deployment, &request_region);
    let headers = header_map(&preferred);
    assert!(
        headers
            .get("Authorization")
            .is_some_and(|value| value.contains("/us-west-2/bedrock/aws4_request"))
    );
    assert_eq!(
        preferred.url,
        "https://bedrock.us-east-1.amazonaws.com/model-invocation-job/arn%3Aaws%3Abedrock%3Aus-east-1%3A123456789012%3Amodel-invocation-job%2Fjob-abc"
    );
    let china = signed_get(
        "arn:aws-cn:bedrock:cn-north-1:1:model-invocation-job/x",
        &deployment,
        &Params::new(),
    );
    assert_eq!(
        china.url,
        "https://bedrock.cn-north-1.amazonaws.com.cn/model-invocation-job/arn%3Aaws-cn%3Abedrock%3Acn-north-1%3A1%3Amodel-invocation-job%2Fx"
    );
}

fn signing_region_from_params() {
    let deployment = params(&[
        ("aws_access_key_id", "AKIDEXAMPLE"),
        ("aws_secret_access_key", SECRET),
    ]);
    let request_region = params(&[("aws_region_name", "eu-west-1")]);
    let signed = signed_get(JOB_ARN, &deployment, &request_region);
    let headers = header_map(&signed);
    assert!(
        headers
            .get("Authorization")
            .is_some_and(|value| value.contains("/eu-west-1/bedrock/aws4_request")),
        "{headers:?}"
    );
}

fn malformed_signing_params() {
    let mut malformed = params(&[("aws_secret_access_key", SECRET)]);
    malformed.insert(
        "aws_access_key_id".to_owned(),
        RequestValue::Mapping(IndexMap::new()),
    );
    let empty = Params::new();
    let attempt = retrieve_signed_request(
        RetrieveRequest {
            batch_id: JOB_ARN,
            litellm_params: &malformed,
            optional_params: &empty,
            now: signing_now(),
        },
        |_settings, _region| ready(Ok::<Credentials, CredentialError>(credentials())),
    );
    assert!(matches!(
        runtime().block_on(attempt),
        Err(BatchStatusError::Signing { .. })
    ));
}

fn retrieve(batch_id: &str, deployment: &Params) -> Result<AwsSignedRequest, BatchStatusError> {
    let empty = Params::new();
    let retrieve = retrieve_signed_request(
        RetrieveRequest {
            batch_id,
            litellm_params: deployment,
            optional_params: &empty,
            now: signing_now(),
        },
        |_settings, _region| ready(Ok::<Credentials, CredentialError>(credentials())),
    );
    runtime().block_on(retrieve)
}

fn arn_validation_errors() {
    let deployment = params(&[("aws_region_name", "us-west-2")]);
    assert!(matches!(
        retrieve("not-an-arn", &deployment),
        Err(BatchStatusError::BatchId { .. })
    ));
    assert!(matches!(
        retrieve("arn:aws:bedrock:us-east-1:acct", &deployment),
        Err(BatchStatusError::ArnFormat { .. })
    ));
    assert!(matches!(
        retrieve(
            "arn:aws:bedrock:US-EAST-1:acct:model-invocation-job/x",
            &deployment
        ),
        Err(BatchStatusError::Region { .. })
    ));
    assert!(matches!(
        retrieve(
            "arn:aws:bedrock:1east:acct:model-invocation-job/x",
            &deployment
        ),
        Err(BatchStatusError::Region { .. })
    ));
}

fn file_config_shapes() {
    file_config_present();
    file_config_broken();
}

fn file_config_present() {
    let full = decode(
        r#"{"jobArn":"a","inputDataConfig":{"s3InputDataConfig":{"s3Uri":"s3://bucket/in.jsonl"}},"outputDataConfig":{"s3OutputDataConfig":{"s3Uri":"s3://bucket/out"}}}"#,
    )
    .expect("the job decodes");
    assert_eq!(full.input_file_id, "s3://bucket/in.jsonl");
    assert_eq!(full.output_file_id.as_deref(), Some("s3://bucket/out"));
    let prediction_prefix = decode(
        r#"{"jobArn":"arn:aws:bedrock:us-east-1:1:model-invocation-job/job","inputDataConfig":{"s3InputDataConfig":{"s3Uri":"s3://bucket/in.jsonl"}},"outputDataConfig":{"s3OutputDataConfig":{"s3Uri":"s3://bucket/out"}}}"#,
    )
    .expect("the job decodes");
    assert_eq!(
        prediction_prefix.output_file_id.as_deref(),
        Some("s3://bucket/out"),
        "the generic projection keeps the supplied prefix"
    );
    let missing_uri = decode(
        r#"{"jobArn":"a","inputDataConfig":{"s3InputDataConfig":{}},"outputDataConfig":{"s3OutputDataConfig":{}}}"#,
    )
    .expect("the job decodes");
    assert_eq!(missing_uri.input_file_id, "");
    assert_eq!(missing_uri.output_file_id.as_deref(), Some(""));
}

fn file_config_broken() {
    let missing_nested =
        decode(r#"{"jobArn":"a","inputDataConfig":{},"outputDataConfig":{}}"#).expect("decodes");
    assert_eq!(missing_nested.input_file_id, "");
    assert_eq!(missing_nested.output_file_id.as_deref(), Some(""));
    let absent = decode(r#"{"jobArn":"a"}"#).expect("the job decodes");
    assert_eq!(absent.input_file_id, "");
    assert_eq!(absent.output_file_id.as_deref(), Some(""));
    let non_dict = decode(r#"{"jobArn":"a","inputDataConfig":null,"outputDataConfig":"prefix"}"#)
        .expect("the job decodes");
    assert_eq!(non_dict.input_file_id, "");
    assert_eq!(non_dict.output_file_id, None);
    let non_dict_nested = decode(
        r#"{"jobArn":"a","inputDataConfig":{"s3InputDataConfig":"uri"},"outputDataConfig":{"s3OutputDataConfig":["uri"]}}"#,
    )
    .expect("the job decodes");
    assert_eq!(non_dict_nested.input_file_id, "");
    assert_eq!(non_dict_nested.output_file_id, None);
    assert!(matches!(
        decode(r#"{"jobArn":"a","inputDataConfig":{"s3InputDataConfig":{"s3Uri":null}}}"#),
        Err(BatchStatusError::Field { field: "s3Uri" })
    ));
    assert!(matches!(
        decode(r#"{"jobArn":"a","outputDataConfig":{"s3OutputDataConfig":{"s3Uri":5}}}"#),
        Err(BatchStatusError::Field { field: "s3Uri" })
    ));
    let null_output =
        decode(r#"{"jobArn":"a","outputDataConfig":{"s3OutputDataConfig":{"s3Uri":null}}}"#)
            .expect("the job decodes");
    assert_eq!(null_output.output_file_id, None);
}

fn handler_region_and_time() {
    region_extraction();
    epoch_json_values();
    epoch_float_and_datetime_values();
    assert_eq!(SIGNATURE_VERSION, "v4");
}

fn region_extraction() {
    assert_eq!(
        region_from_bedrock_arn(JOB_ARN).as_deref(),
        Some("us-east-1")
    );
    assert_eq!(
        region_from_bedrock_arn("arn:aws:bedrock:us-east-1").as_deref(),
        Some("us-east-1")
    );
    assert_eq!(region_from_bedrock_arn("arn:aws:bedrock::acct:x"), None);
    assert_eq!(region_from_bedrock_arn("arn:aws:s3:us-east-1:acct:x"), None);
    assert_eq!(region_from_bedrock_arn("arn:aws:bedrock"), None);
    assert_eq!(region_from_bedrock_arn(""), None);
    assert_eq!(region_from_bedrock_arn("garbage"), None);
    assert_eq!(
        region_from_bedrock_arn("arn:aws:s3-bedrock:us-east-1:acct:x"),
        None
    );
}

fn epoch_json_values() {
    let integer = json("5");
    assert_eq!(to_epoch(Some(EpochInput::Json(&integer))).unwrap(), Some(5));
    let fraction = json("1.9");
    assert_eq!(
        to_epoch(Some(EpochInput::Json(&fraction))).unwrap(),
        Some(1)
    );
    let negative = json("-1.9");
    assert_eq!(
        to_epoch(Some(EpochInput::Json(&negative))).unwrap(),
        Some(-1)
    );
    let flag = json("true");
    assert_eq!(to_epoch(Some(EpochInput::Json(&flag))).unwrap(), Some(1));
    let text = json("\"5\"");
    assert_eq!(to_epoch(Some(EpochInput::Json(&text))).unwrap(), None);
    let null = json("null");
    assert_eq!(to_epoch(Some(EpochInput::Json(&null))).unwrap(), None);
    let list = json("[5]");
    assert_eq!(to_epoch(Some(EpochInput::Json(&list))).unwrap(), None);
    assert_eq!(to_epoch(None).unwrap(), None);
}

fn epoch_float_and_datetime_values() {
    let moment = DateTime::parse_from_rfc3339("2024-01-15T10:30:00.5Z")
        .unwrap()
        .with_timezone(&Utc);
    assert_eq!(
        to_epoch(Some(EpochInput::DateTime(moment))).unwrap(),
        Some(1_705_314_600)
    );
    let earlier = DateTime::parse_from_rfc3339("1969-12-31T23:59:59.5Z")
        .unwrap()
        .with_timezone(&Utc);
    assert_eq!(
        to_epoch(Some(EpochInput::DateTime(earlier))).unwrap(),
        Some(0)
    );
    assert_eq!(to_epoch(Some(EpochInput::Float(1.9))).unwrap(), Some(1));
    assert!(matches!(
        to_epoch(Some(EpochInput::Float(f64::INFINITY))),
        Err(BatchStatusError::NotFinite)
    ));
    assert!(matches!(
        to_epoch(Some(EpochInput::Float(f64::NAN))),
        Err(BatchStatusError::NotFinite)
    ));
    assert!(matches!(
        to_epoch(Some(EpochInput::Float(1e300))),
        Err(BatchStatusError::EpochRange)
    ));
}
