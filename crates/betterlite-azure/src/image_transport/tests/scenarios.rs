use std::collections::BTreeMap;

use crate::image_transport::error::{FailureMessage, ImageTransportError};
use crate::image_transport::request::TimeoutPhases;

use super::support::{
    API_KEY, DEFAULT_TIMEOUT, DEPLOYMENT_BASE, OPERATION, Outcome, PREVIEW_QUERY_BASE,
    PREVIEW_VERSION, PROVIDER_BASE, Reply, SUBMIT_QUERY_URL, SUBMIT_URL, SYNTHETIC_URL, Step,
    V1_BASE, data_json, failure_of, json, operation_error, parsed, polling_timeout, reply,
    request_metadata, run, success, text,
};

const RUNNING: &str = r#"{"status":"running"}"#;
const SUCCEEDED: &str =
    r#"{"status":"succeeded","result":{"data":[{"url":"https://example.test/image.png"}]}}"#;
const OPENAI_IMAGE: &str = r#"{"created":1,"data":[{"url":"https://example.test/image.png"}]}"#;
const MODEL_BODY: &str = r#"{"model":"dall-e-2","prompt":"a cat","n":1}"#;
const PROMPT_BODY: &str = r#"{"prompt":"a cat","n":1}"#;
const DEFAULT_TIMEOUT_PHASES: TimeoutPhases = TimeoutPhases {
    connect: 5.0,
    read: 600.0,
    write: 600.0,
    pool: 600.0,
};

fn submitted() -> Vec<Reply> {
    vec![submit_reply(), reply(200, vec![], SUCCEEDED)]
}

fn polled(mut replies: Vec<Reply>) -> Outcome {
    replies.insert(0, submit_reply());
    run(Step::preview(replies))
}

fn assert_single_post(step: Step, expected_body: &str) {
    let api_base = step.api_base;
    let outcome = run(step);
    assert_eq!(success(&outcome).status, 200);
    let calls = outcome.harness.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url, api_base);
    assert_eq!(calls[0].body, Some(json(expected_body)));
    assert_eq!(data_json(&outcome.data), json(MODEL_BODY));
}

fn direct(api_base: &'static str, deployment_name: Option<&'static str>) -> Step {
    Step::direct(
        api_base,
        deployment_name,
        vec![reply(200, vec![], OPENAI_IMAGE)],
    )
}

fn submit_reply() -> Reply {
    reply(
        202,
        vec![("operation-location", OPERATION)],
        r#"{"id":"op-1"}"#,
    )
}

pub(super) fn image_immediate_result() {
    let outcome = run(Step::direct(
        PROVIDER_BASE,
        None,
        vec![reply(
            200,
            vec![("x-ms-request-id", "req-direct")],
            OPENAI_IMAGE,
        )],
    ));
    let response = success(&outcome);
    assert_eq!(response.status, 200);
    assert_eq!(text(&response.headers, "x-ms-request-id"), "req-direct");
    assert_eq!(parsed(&response.body), json(OPENAI_IMAGE));
    assert_eq!(
        request_metadata(response),
        ("POST".to_owned(), PROVIDER_BASE.to_owned())
    );
    let calls = outcome.harness.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "POST");
    assert_eq!(calls[0].url, PROVIDER_BASE);
    assert_eq!(calls[0].header("api-key"), Some(API_KEY));
    assert_eq!(calls[0].body, Some(json(MODEL_BODY)));
    assert_eq!(outcome.harness.constructed(), vec![DEFAULT_TIMEOUT_PHASES]);
    assert_eq!(data_json(&outcome.data), json(MODEL_BODY));
    assert!(outcome.harness.sleeps().is_empty());
}

pub(super) fn operation_location_poll() {
    timeout_terminal_after_deadline();
    let outcome = run(Step::preview(vec![
        submit_reply(),
        reply(200, vec![("retry-after", "0")], RUNNING),
        reply(200, vec![("x-ms-request-id", "req-poll")], SUCCEEDED),
    ]));
    let response = success(&outcome);
    assert_eq!(response.status, 200);
    assert_eq!(text(&response.headers, "x-ms-request-id"), "req-poll");
    assert_eq!(
        parsed(&response.body),
        json(r#"{"data":[{"url":"https://example.test/image.png"}]}"#)
    );
    assert_eq!(
        request_metadata(response),
        ("POST".to_owned(), SYNTHETIC_URL.to_owned())
    );
    let calls = outcome.harness.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].method, "POST");
    assert_eq!(calls[0].url, SUBMIT_URL);
    assert_eq!(calls[0].body, Some(json(PROMPT_BODY)));
    assert_eq!(calls[1].method, "GET");
    assert_eq!(calls[1].url, OPERATION);
    assert_eq!(calls[2].url, OPERATION);
    assert_eq!(outcome.harness.sleeps(), vec![0]);
}

pub(super) fn retry_after_poll() {
    let outcome = polled(vec![
        reply(200, vec![("retry-after", "2")], RUNNING),
        reply(200, vec![("retry-after", "3")], RUNNING),
        reply(200, vec![], SUCCEEDED),
    ]);
    assert_eq!(success(&outcome).status, 200);
    assert_eq!(outcome.harness.sleeps(), vec![2, 3]);
    let calls = outcome.harness.calls();
    assert_eq!(calls.len(), 4);
    assert!(
        calls[1..]
            .iter()
            .all(|call| call.method == "GET" && call.url == OPERATION)
    );
}

pub(super) fn poll_default_timeout_120() {
    assert_eq!(polling_timeout(&BTreeMap::new()), DEFAULT_TIMEOUT);
    let outcome = run(Step::preview(vec![
        submit_reply(),
        reply(200, vec![("retry-after", "120")], RUNNING),
        reply(200, vec![("retry-after", "0")], RUNNING),
        Reply {
            delay: 1,
            ..reply(200, vec![], RUNNING)
        },
    ]));
    let failure = failure_of(&outcome);
    assert!(matches!(failure, ImageTransportError::PollingTimeout));
    assert_eq!(failure.status_code(), Some(408));
    assert_eq!(outcome.harness.sleeps(), vec![120, 0]);
    assert_eq!(outcome.harness.calls().len(), 4);
}

pub(super) fn retry_after_default_10() {
    let outcome = run(Step::preview(vec![
        submit_reply(),
        reply(200, vec![], RUNNING),
        reply(200, vec![], SUCCEEDED),
    ]));
    assert_eq!(success(&outcome).status, 200);
    assert_eq!(outcome.harness.sleeps(), vec![10]);
}

pub(super) fn same_origin_502() {
    let outcome = run(Step::preview(vec![reply(
        202,
        vec![(
            "operation-location",
            "https://evil.test/openai/operations/op-1",
        )],
        r#"{"id":"op-1"}"#,
    )]));
    let failure = failure_of(&outcome);
    assert!(matches!(
        failure,
        ImageTransportError::RejectedPollingUrl { .. }
    ));
    assert_eq!(failure.status_code(), Some(502));
    assert!(failure.to_string().contains("Rejected polling URL"));
    assert_eq!(outcome.harness.calls().len(), 1);
    assert!(outcome.harness.sleeps().is_empty());
}

pub(super) fn missing_operation_location_500() {
    let outcome = run(Step::preview(vec![reply(500, vec![], "upstream exploded")]));
    let failure = failure_of(&outcome);
    assert!(matches!(
        failure,
        ImageTransportError::MissingOperationLocation { .. }
    ));
    assert_eq!(failure.status_code(), Some(500));
    assert!(failure.to_string().contains("upstream exploded"));
    assert_eq!(outcome.harness.calls().len(), 1);
}

pub(super) fn missing_status_502() {
    let initial = polled(vec![reply(200, vec![], r#"{"result":{}}"#)]);
    let first = failure_of(&initial);
    assert!(matches!(first, ImageTransportError::MissingStatusInitial));
    assert_eq!(first.status_code(), Some(502));
    assert_eq!(initial.harness.calls().len(), 2);

    let later = polled(vec![
        reply(200, vec![], RUNNING),
        reply(200, vec![], r#"{"result":{}}"#),
    ]);
    let second = failure_of(&later);
    assert!(matches!(second, ImageTransportError::MissingStatus));
    assert_eq!(second.status_code(), None);
    assert_eq!(later.harness.calls().len(), 3);
    assert_eq!(later.harness.sleeps(), vec![10]);

    let invalid = polled(vec![reply(200, vec![], "not json")]);
    let third = failure_of(&invalid);
    assert!(matches!(third, ImageTransportError::Decode(_)));
    assert_eq!(third.status_code(), None);
}

pub(super) fn polling_timeout_408() {
    timeout_strictly_greater();
    timeout_terminal_after_deadline();
}

fn timeout_strictly_greater() {
    let outcome = run(Step::preview(vec![
        submit_reply(),
        reply(200, vec![("retry-after", "10")], RUNNING),
        reply(200, vec![("retry-after", "0")], RUNNING),
        Reply {
            delay: 1,
            ..reply(200, vec![], RUNNING)
        },
    ])
    .with_timeout("10"));
    let failure = failure_of(&outcome);
    assert!(matches!(failure, ImageTransportError::PollingTimeout));
    assert_eq!(failure.status_code(), Some(408));
    assert_eq!(outcome.harness.sleeps(), vec![10, 0]);
    assert_eq!(outcome.harness.calls().len(), 4);
}

pub(super) fn retry_after_capped_at_deadline() {
    for (sync, header, last, succeeds) in [
        (false, "86400", RUNNING, false),
        (true, "21", RUNNING, false),
        (false, "86400", SUCCEEDED, true),
        (true, "21", SUCCEEDED, true),
    ] {
        let mut step = Step::preview(vec![
            submit_reply(),
            reply(200, vec![("retry-after", "100")], RUNNING),
            reply(200, vec![("retry-after", header)], RUNNING),
            reply(200, vec![], last),
        ]);
        if sync {
            step = step.sync();
        }
        let outcome = run(step);
        if succeeds {
            assert_eq!(success(&outcome).status, 200);
        } else {
            let failure = failure_of(&outcome);
            assert!(matches!(failure, ImageTransportError::PollingTimeout));
            assert_eq!(failure.status_code(), Some(408));
        }
        assert_eq!(outcome.harness.sleeps(), vec![100, 20]);
        assert_eq!(outcome.harness.elapsed_seconds(), 120);
        assert_eq!(outcome.harness.calls().len(), 4);
    }
}

fn timeout_terminal_after_deadline() {
    let outcome = run(Step::preview(vec![
        submit_reply(),
        Reply {
            delay: 5,
            ..reply(200, vec![("retry-after", "0")], RUNNING)
        },
        reply(200, vec![], SUCCEEDED),
    ])
    .with_timeout("0"));
    assert_eq!(success(&outcome).status, 200);
    assert_eq!(outcome.harness.elapsed_seconds(), 5);
    assert_eq!(outcome.harness.sleeps(), vec![0]);
    assert_eq!(outcome.harness.calls().len(), 3);
}

pub(super) fn failed_operation_400() {
    failed_with_message();
    let cases = [
        (
            r#"{"status":"failed","error":{"code":"x"}}"#,
            "Image generation failed",
            "image_generation_failed",
        ),
        (
            r#"{"status":"failed","error":"boom"}"#,
            r#"{"status":"failed","error":"boom"}"#,
            "operation_json",
        ),
        (
            r#"{"status":"failed","code":"y"}"#,
            "Image generation failed",
            "image_generation_failed",
        ),
    ];
    for (operation, expected, kind) in cases {
        let outcome = polled(vec![reply(200, vec![], operation)]);
        let (body, message) = operation_error(failure_of(&outcome));
        assert_eq!(message, expected);
        assert_eq!(*body, json(operation));
        assert_eq!(FailureMessage::of(body).as_str(), kind);
    }
}

fn failed_with_message() {
    let outcome = run(Step::preview(vec![
        submit_reply(),
        reply(
            200,
            vec![],
            r#"{"status":"failed","error":{"message":"content policy","code":"content_policy_violation"}}"#,
        ),
    ]));
    let failure = failure_of(&outcome);
    assert!(matches!(
        failure,
        ImageTransportError::OperationFailed { .. }
    ));
    assert_eq!(failure.status_code(), Some(400));
    let (body, message) = operation_error(failure);
    assert_eq!(message, "content policy");
    assert_eq!(FailureMessage::of(body).as_str(), "error_message");
    assert_eq!(
        *body,
        json(
            r#"{"status":"failed","error":{"message":"content policy","code":"content_policy_violation"}}"#
        )
    );
}

pub(super) fn retry_after_not_integer() {
    retry_after_non_integer();
    retry_after_empty_and_negative();
    retry_after_negative_sync();
}

fn retry_after_non_integer() {
    let outcomes = ["soon", "\u{e9}"]
        .map(|header| polled(vec![reply(200, vec![("retry-after", header)], RUNNING)]));
    for outcome in &outcomes {
        let failure = failure_of(outcome);
        assert!(matches!(failure, ImageTransportError::RetryAfter { .. }));
        assert_eq!(failure.status_code(), None);
        assert_eq!(outcome.harness.calls().len(), 2);
        assert!(outcome.harness.sleeps().is_empty());
    }
}

fn retry_after_empty_and_negative() {
    for (header, slept, elapsed) in [("", 10, 10), ("-5", -5, 0)] {
        let outcome = polled(vec![
            reply(200, vec![("retry-after", header)], RUNNING),
            reply(200, vec![], SUCCEEDED),
        ]);
        assert_eq!(success(&outcome).status, 200);
        assert_eq!(outcome.harness.sleeps(), vec![slept]);
        assert_eq!(outcome.harness.elapsed_seconds(), elapsed);
    }
}

fn retry_after_negative_sync() {
    let zero = run(Step::preview(vec![
        submit_reply(),
        reply(200, vec![("retry-after", "0")], RUNNING),
        reply(200, vec![], SUCCEEDED),
    ])
    .sync());
    assert_eq!(success(&zero).status, 200);
    assert_eq!(zero.harness.sleeps(), vec![0]);
    let outcome = run(Step::preview(vec![
        submit_reply(),
        reply(200, vec![("retry-after", "-5")], RUNNING),
    ])
    .sync());
    let failure = failure_of(&outcome);
    assert!(matches!(
        failure,
        ImageTransportError::NegativeSleep { seconds: -5 }
    ));
    assert_eq!(failure.status_code(), None);
    assert!(outcome.harness.sleeps().is_empty());
}

pub(super) fn preview_submit_gate() {
    preview_gate_polls();
    preview_gate_direct();
}

fn preview_gate_polls() {
    let mut step = Step::preview(vec![submit_reply(), reply(200, vec![], SUCCEEDED)]);
    step.api_base = PREVIEW_QUERY_BASE;
    let outcome = run(step);
    assert_eq!(success(&outcome).status, 200);
    let calls = outcome.harness.calls();
    assert_eq!(calls[0].url, SUBMIT_QUERY_URL);
    assert_eq!(calls[0].body, Some(json(PROMPT_BODY)));
    assert_eq!(data_json(&outcome.data), json(PROMPT_BODY));
}

fn preview_gate_direct() {
    assert_single_post(direct(DEPLOYMENT_BASE, Some("dall-e-2")), PROMPT_BODY);
    let mut newer = direct(DEPLOYMENT_BASE, Some("dall-e-2"));
    newer.api_version = "2023-11-01-preview";
    assert_single_post(newer, PROMPT_BODY);
    let mut other = direct("https://example.test/openai/v1/chat/completions", None);
    other.api_version = PREVIEW_VERSION;
    assert_single_post(other, MODEL_BODY);
}

pub(super) fn deployment_body_model_drop() {
    assert_single_post(direct(DEPLOYMENT_BASE, Some("dall-e-2")), PROMPT_BODY);
    assert_single_post(direct(DEPLOYMENT_BASE, None), PROMPT_BODY);
}

pub(super) fn v1_body_deployment_model() {
    assert_single_post(
        direct(V1_BASE, Some("dall-e-3")),
        r#"{"model":"dall-e-3","prompt":"a cat","n":1}"#,
    );
    assert_single_post(direct(V1_BASE, None), MODEL_BODY);
}

pub(super) fn provider_body_unchanged() {
    assert_single_post(direct(PROVIDER_BASE, None), MODEL_BODY);
    assert_single_post(direct(PROVIDER_BASE, Some("dall-e-3")), MODEL_BODY);
}

pub(super) fn http_timeout_defaults() {
    timeout_default();
    timeout_numeric();
    timeout_supplied_client();
}

fn timeout_default() {
    let outcome = run(Step::preview(submitted()));
    assert_eq!(success(&outcome).status, 200);
    assert_eq!(outcome.harness.constructed(), vec![DEFAULT_TIMEOUT_PHASES]);
}

fn timeout_numeric() {
    let mut step = Step::preview(submitted());
    step.timeout_argument = Some(5.0_f64);
    let outcome = run(step);
    assert_eq!(success(&outcome).status, 200);
    assert_eq!(
        outcome.harness.constructed(),
        vec![TimeoutPhases {
            connect: 5.0,
            read: 5.0,
            write: 5.0,
            pool: 5.0,
        }]
    );
}

fn timeout_supplied_client() {
    let mut step = Step::preview(submitted());
    step.supplied_client = true;
    step.timeout_argument = Some(5.0_f64);
    let outcome = run(step);
    assert_eq!(success(&outcome).status, 200);
    assert!(outcome.harness.constructed().is_empty());
}
