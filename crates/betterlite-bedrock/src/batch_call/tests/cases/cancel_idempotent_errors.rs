use betterlite_openai::batches::records::BatchStatus;

use crate::batch_call::error::BatchCallError;

use super::super::fixture::{JOB_ID, REGION, answer, answer_with, auth_parameters, run_job};

const STOPPED: &str = r#"{"jobArn":"arn:aws:bedrock:eu-west-1:123456789012:model-invocation-job/job-1","status":"Stopped"}"#;
const CANCELLING: &str = r#"{"jobArn":"arn:aws:bedrock:eu-west-1:123456789012:model-invocation-job/job-1","status":"Stopping"}"#;
const IN_PROGRESS: &str = r#"{"jobArn":"arn:aws:bedrock:eu-west-1:123456789012:model-invocation-job/job-1","status":"InProgress"}"#;
const EXPIRED: &str = r#"{"jobArn":"arn:aws:bedrock:eu-west-1:123456789012:model-invocation-job/job-1","status":"Expired"}"#;
const ASYNC_ARN: &str = "arn:aws:bedrock:eu-west-1:123456789012:async-invoke/abc";
const ENCODED_ASYNC_ARN: &str =
    "arn%3Aaws%3Abedrock%3Aeu-west-1%3A123456789012%3Aasync-invoke%2Fabc";

pub(super) fn cancel_idempotent_errors() {
    tolerated_returns_the_lookup();
    tolerated_but_still_running();
    untolerated_code_is_rethrown();
    non_text_codes_are_kept();
    failed_lookup_is_the_error();
    error_code_precedence();
    status_codes();
    three_hundred_is_an_error();
    async_identifier_stops_the_same_way();
}

fn tolerated_returns_the_lookup() {
    let validation = answer_with(400, &[("x-amzn-errortype", "ValidationException")], "");
    let conflict = answer_with(409, &[("x-amzn-errortype", "ConflictException")], "");
    let suffixed = answer_with(
        400,
        &[("x-amzn-errortype", "ConflictException:http://internal")],
        "",
    );
    let typed = answer(
        400,
        r#"{"__type":"aws.protocol#ValidationException:http://internal"}"#,
    );
    let runs = [
        (validation, STOPPED, BatchStatus::Cancelled),
        (conflict, CANCELLING, BatchStatus::Cancelling),
        (suffixed, EXPIRED, BatchStatus::Expired),
        (typed, STOPPED, BatchStatus::Cancelled),
    ];
    for (stop, body, expected) in runs {
        let run = run_job(
            vec![stop, answer(200, body)],
            JOB_ID,
            Some(REGION),
            &auth_parameters(),
        );
        let batch = run
            .result
            .expect("an idempotent stop error returns the current status");
        assert_eq!(batch.status, expected);
        assert_eq!(run.factory.requests().len(), 2);
    }
}

fn tolerated_but_still_running() {
    let stop = answer_with(409, &[("x-amzn-errortype", "ConflictException")], "");
    let run = run_job(
        vec![stop, answer(200, IN_PROGRESS)],
        JOB_ID,
        Some(REGION),
        &auth_parameters(),
    );
    let error = run
        .result
        .expect_err("a stop error over a still running job is rethrown");
    assert_eq!(client_code(&error), Some("ConflictException"));
    assert_eq!(run.factory.requests().len(), 2);
}

fn untolerated_code_is_rethrown() {
    let stop = answer(400, r#"{"code":"AccessDeniedException"}"#);
    let run = run_job(vec![stop], JOB_ID, Some(REGION), &auth_parameters());
    let error = run
        .result
        .expect_err("a stop error outside the tolerated codes is rethrown");
    assert_eq!(client_code(&error), Some("AccessDeniedException"));
    assert_eq!(run.factory.requests().len(), 1);
}

// a code of a non-text type keeps its Python str() spelling instead of one of the tolerated names
fn non_text_codes_are_kept() {
    let runs = [
        (r#"{"code":true}"#, Some("True")),
        (r#"{"code":5}"#, Some("5")),
    ];
    for (body, expected) in runs {
        let run = run_job(
            vec![answer(400, body)],
            JOB_ID,
            Some(REGION),
            &auth_parameters(),
        );
        let error = run
            .result
            .expect_err("an untolerated stop error is rethrown");
        assert_eq!(client_code(&error), expected);
        assert_eq!(run.factory.requests().len(), 1);
    }
}

fn failed_lookup_is_the_error() {
    let stop = answer_with(400, &[("x-amzn-errortype", "ValidationException")], "");
    let lookup = answer(500, r#"{"code":"InternalServerException"}"#);
    let run = run_job(vec![stop, lookup], JOB_ID, Some(REGION), &auth_parameters());
    let error = run
        .result
        .expect_err("a failed status lookup replaces the stop error");
    assert_eq!(client_code(&error), Some("InternalServerException"));
    assert_eq!(run.factory.requests().len(), 2);
}

// the x-amzn-errortype header wins over a code in the body, and the body's code wins over __type
fn error_code_precedence() {
    let header = answer_with(
        400,
        &[("x-amzn-errortype", "ValidationException")],
        r#"{"code":"AccessDeniedException"}"#,
    );
    let body = answer(
        400,
        r#"{"code":"ConflictException","__type":"AccessDeniedException"}"#,
    );
    let upper_key = answer(400, r#"{"Code":"ValidationException"}"#);
    let runs = [header, body, upper_key];
    for stop in runs {
        let run = run_job(
            vec![stop, answer(200, STOPPED)],
            JOB_ID,
            Some(REGION),
            &auth_parameters(),
        );
        let batch = run
            .result
            .expect("the tolerated error code returns the current status");
        assert_eq!(batch.status, BatchStatus::Cancelled);
    }
}

fn status_codes() {
    let cases = [
        (answer(503, ""), Some("503"), 503_u16),
        (
            answer(502, "<html>bad gateway</html>"),
            Some("502"),
            502_u16,
        ),
        (answer(400, r#"{"__type":null}"#), Some(""), 400_u16),
        (answer(301, ""), Some("301"), 301_u16),
    ];
    for (stop, code, status) in cases {
        let run = run_job(vec![stop], JOB_ID, Some(REGION), &auth_parameters());
        let error = run
            .result
            .expect_err("an untolerated stop error is rethrown");
        assert_eq!(client_code(&error), code);
        assert!(matches!(
            &error,
            BatchCallError::Client { status: actual, .. } if *actual == status
        ));
    }
}

fn three_hundred_is_an_error() {
    let run = run_job(
        vec![answer(300, "")],
        JOB_ID,
        Some(REGION),
        &auth_parameters(),
    );
    let error = run
        .result
        .expect_err("a 300 reply is a client error without a code");
    assert_eq!(client_code(&error), None);
    assert_eq!(run.factory.requests().len(), 1);
}

fn async_identifier_stops_the_same_way() {
    let run = run_job(
        vec![answer(200, ""), answer(200, CANCELLING)],
        ASYNC_ARN,
        Some(REGION),
        &auth_parameters(),
    );
    let batch = run
        .result
        .expect("an async ARN cancels through the same stop operation");
    assert_eq!(batch.status, BatchStatus::Cancelling);
    let requests = run.factory.requests();
    assert_eq!(requests[0].method, "POST");
    assert_eq!(
        requests[0].url,
        format!(
            "https://bedrock.{REGION}.amazonaws.com/model-invocation-job/{ENCODED_ASYNC_ARN}/stop"
        )
    );
}

fn client_code(error: &BatchCallError) -> Option<&str> {
    if let BatchCallError::Client { code, .. } = error {
        return code.as_deref();
    }
    None
}
