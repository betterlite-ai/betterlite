#![cfg(test)]

mod support;

use std::time::Duration;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use rstest::rstest;

use self::support::{
    Answer, Bench, Call, IMPATIENT, assert_line, assert_no_request, body_json, json, json_server,
    object, provider, run, url,
};
use crate::api::eval_http::error::EvalHttpError;

const TIMEOUT: Duration = Duration::from_millis(50);

const RUN: &str = r#"{"id":"run_1","object":"eval.run","created_at":1700000000,"status":"queued","data_source":{},"eval_id":"eval_1"}"#;
const PAGE: &str = r#"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#;
const CANCELLED: &str = r#"{"id":"run_1","object":"eval.run","status":"cancelled"}"#;
const DELETED: &str = r#"{"run_id":"run_1","object":"eval.run.deleted","deleted":true}"#;
const CREATE_BODY: &str =
    r#"{"name":"my run","data_source":{"type":"jsonl","source":{"type":"file_id","id":"file-1"}}}"#;
const LIST_QUERY: &str = r#"{"limit":5,"order":"asc"}"#;

const CREATE_URL: &str = "/v1/evals/eval_1/runs";
const RUN_URL: &str = "/v1/evals/eval_1/runs/run_1";
const CANCEL_URL: &str = "/v1/evals/eval_1/runs/run_1/cancel";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handler {
    CreateRun,
    ListRuns,
    GetRun,
    CancelRun,
    DeleteRun,
}

// What one handler puts on the wire and which response transform reads its answer
struct Expectation {
    method: &'static str,
    path: &'static str,
    body: Option<&'static str>,
    reply: &'static str,
    transform: &'static str,
}

#[rustfmt::skip]
const fn expectation(handler: Handler) -> Expectation {
    match handler {
        Handler::CreateRun => Expectation { method: "POST", path: CREATE_URL, body: Some(CREATE_BODY), reply: RUN, transform: "create_run_response" },
        Handler::ListRuns => Expectation { method: "GET", path: "/v1/evals/eval_1/runs?limit=5&order=asc", body: None, reply: PAGE, transform: "list_runs_response" },
        Handler::GetRun => Expectation { method: "GET", path: RUN_URL, body: None, reply: RUN, transform: "get_run_response" },
        Handler::CancelRun => Expectation { method: "POST", path: CANCEL_URL, body: Some("{}"), reply: CANCELLED, transform: "cancel_run_response" },
        Handler::DeleteRun => Expectation { method: "DELETE", path: RUN_URL, body: None, reply: DELETED, transform: "delete_run_response" },
    }
}

// The URL the operation hands the handler: the transform's own path, with the query travelling as params instead
#[rustfmt::skip]
const fn handler_url(handler: Handler) -> &'static str {
    match handler {
        Handler::CreateRun | Handler::ListRuns => CREATE_URL,
        Handler::GetRun | Handler::DeleteRun => RUN_URL,
        Handler::CancelRun => CANCEL_URL,
    }
}

// The call a handler's case runs, with the body or query that handler's request carries
#[rustfmt::skip]
fn call<'call>(handler: Handler, body: &'call Object, query: &'call Object, timeout: Option<Duration>) -> Call<'call> {
    match handler {
        Handler::CreateRun => Call::Create { timeout, body },
        Handler::ListRuns => Call::List { query },
        Handler::GetRun => Call::Get,
        Handler::CancelRun => Call::Cancel { timeout },
        Handler::DeleteRun => Call::Delete { timeout },
    }
}

fn assert_answer(handler: Handler, answered: Answer) {
    match answered {
        Answer::Run(run) => {
            assert!(matches!(handler, Handler::CreateRun | Handler::GetRun));
            assert_eq!(run.id, "run_1");
        }
        Answer::Runs(page) => {
            assert_eq!(handler, Handler::ListRuns);
            assert!(page.data.is_empty());
        }
        Answer::Cancelled(cancelled) => {
            assert_eq!(handler, Handler::CancelRun);
            assert_eq!(cancelled.id, "run_1");
        }
        Answer::Deleted(deleted) => {
            assert_eq!(handler, Handler::DeleteRun);
            assert_eq!(deleted.run_id, "run_1");
        }
    }
}

// The method, the path, the query and the body each handler puts on the wire, and the response transform that reads the reply
#[rstest]
#[case::create(Handler::CreateRun)]
#[case::list(Handler::ListRuns)]
#[case::get(Handler::GetRun)]
#[case::cancel(Handler::CancelRun)]
#[case::delete(Handler::DeleteRun)]
#[tokio::test]
async fn handler_sends_litellm_method_and_body(#[case] handler: Handler) {
    let expected = expectation(handler);
    let serving = json_server("200 OK", expected.reply, Duration::ZERO);
    let mut bench = Bench::new(
        url(&serving, handler_url(handler)),
        TransportLimits::DEFAULT,
    );
    let body = object(CREATE_BODY);
    let query = object(LIST_QUERY);
    let answered = run(&mut bench, call(handler, &body, &query, Some(TIMEOUT)))
        .await
        .expect("the handler answers");
    assert_answer(handler, answered);
    assert_eq!(bench.config.seen(), [expected.transform]);
    let request = serving.finish().expect("the fixture finishes");
    assert_line(&request, expected.method, expected.path);
    match expected.body {
        Some(expected_body) => assert_eq!(body_json(&request), json(expected_body)),
        None => assert!(request.body.is_empty()),
    }
    assert!(request.head.contains("x-test: yes"));
    assert_eq!(bench.events.recorded(), ["log"]);
}

// A create, a cancel and a delete send the caller's timeout, while a list and a get keep the client's read deadline
#[rstest]
#[case::create(
    Handler::CreateRun,
    408_u16,
    false,
    "litellm.Timeout: Connection timed out. Timeout passed=0.05, time taken="
)]
#[case::cancel(
    Handler::CancelRun,
    408_u16,
    false,
    "litellm.Timeout: Connection timed out. Timeout passed=0.05, time taken="
)]
#[case::delete(
    Handler::DeleteRun,
    500_u16,
    true,
    "the request ran past its read timeout or total deadline"
)]
#[tokio::test]
async fn the_callers_timeout_reaches_the_send(
    #[case] handler: Handler,
    #[case] code: u16,
    #[case] synthesized: bool,
    #[case] message: &str,
) {
    let expected = expectation(handler);
    let serving = json_server("200 OK", expected.reply, Duration::from_millis(300));
    let mut bench = Bench::new(
        url(&serving, handler_url(handler)),
        TransportLimits::DEFAULT,
    );
    let body = object(CREATE_BODY);
    let query = object(LIST_QUERY);
    let error = run(&mut bench, call(handler, &body, &query, Some(TIMEOUT)))
        .await
        .expect_err("the 50 ms timeout ends the call");
    let request = serving
        .finish()
        .expect("the fixture read the request before its pause");
    assert_line(&request, expected.method, expected.path);
    let provider = provider(&error);
    assert_eq!(provider.status_code, code);
    assert_eq!(provider.status_code_is_synthesized, synthesized);
    if synthesized {
        assert_eq!(provider.message, message);
    } else {
        assert!(
            provider.message.starts_with(message),
            "{}",
            provider.message
        );
        assert!(
            provider.message.ends_with(" seconds"),
            "{}",
            provider.message
        );
    }
}

// list and get pass no timeout, so the pooled client's read deadline is what ends a slow answer
#[rstest]
#[case::list(Handler::ListRuns)]
#[case::get(Handler::GetRun)]
#[tokio::test]
async fn list_and_get_keep_the_clients_deadline(#[case] handler: Handler) {
    let expected = expectation(handler);
    let serving = json_server("200 OK", expected.reply, Duration::from_millis(300));
    let mut bench = Bench::new(url(&serving, handler_url(handler)), IMPATIENT);
    let body = object(CREATE_BODY);
    let query = object(LIST_QUERY);
    let error = run(&mut bench, call(handler, &body, &query, Some(TIMEOUT)))
        .await
        .expect_err("the client's 100 ms deadline ends the call");
    let request = serving
        .finish()
        .expect("the fixture read the request before its pause");
    assert_line(&request, expected.method, expected.path);
    let provider = provider(&error);
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(
        provider.message,
        "the request ran past its read timeout or total deadline"
    );
}

// A cancel 409, a delete 404, the 400 boundary create and an unfollowed 302 all raise in the client, so _handle_error carries the status and text
#[rstest]
#[case::cancel_conflict(
    Handler::CancelRun,
    "409 Conflict",
    r#"{"error":{"message":"the run already finished"}}"#,
    409_u16
)]
#[case::delete_missing(
    Handler::DeleteRun,
    "404 Not Found",
    r#"{"error":{"message":"no such run"}}"#,
    404_u16
)]
#[case::create_bad_request(
    Handler::CreateRun,
    "400 Bad Request",
    r#"{"error":{"message":"the run body is not valid"}}"#,
    400_u16
)]
#[case::create_unfollowed_redirect(
    Handler::CreateRun,
    "302 Found",
    r#"{"error":{"message":"moved"}}"#,
    302_u16
)]
#[tokio::test]
async fn post_and_delete_status_errors_go_through_handle_error(
    #[case] handler: Handler,
    #[case] status: &str,
    #[case] body: &str,
    #[case] code: u16,
) {
    let expected = expectation(handler);
    let serving = json_server(status, body, Duration::ZERO);
    let mut bench = Bench::new(
        url(&serving, handler_url(handler)),
        TransportLimits::DEFAULT,
    );
    let request_body = object(CREATE_BODY);
    let query = object(LIST_QUERY);
    let error = run(
        &mut bench,
        call(handler, &request_body, &query, Some(TIMEOUT)),
    )
    .await
    .expect_err("the upstream status ends the call");
    let request = serving.finish().expect("the fixture finishes");
    assert_line(&request, expected.method, expected.path);
    let provider = provider(&error);
    assert_eq!(provider.status_code, code);
    assert!(!provider.status_code_is_synthesized);
    assert_eq!(provider.message, body);
    let upstream = provider
        .headers
        .as_ref()
        .expect("the upstream headers reach the error");
    assert!(upstream.iter().any(|(name, _)| name == "x-upstream"));
}

// The client never raises for a GET status, so a get 404 and a list 500 reach their transforms with the body they carried
#[rstest]
#[case::get(Handler::GetRun, "404 Not Found", RUN)]
#[case::list(Handler::ListRuns, "500 Internal Server Error", PAGE)]
#[tokio::test]
async fn get_replies_of_any_status_reach_the_transform(
    #[case] handler: Handler,
    #[case] status: &str,
    #[case] body: &str,
) {
    let serving = json_server(status, body, Duration::ZERO);
    let mut bench = Bench::new(
        url(&serving, handler_url(handler)),
        TransportLimits::DEFAULT,
    );
    let request_body = object(CREATE_BODY);
    let query = object(LIST_QUERY);
    let answered = run(
        &mut bench,
        call(handler, &request_body, &query, Some(TIMEOUT)),
    )
    .await
    .expect("a GET reply reaches the transform whatever its status");
    assert_answer(handler, answered);
    serving.finish().expect("the fixture finishes");
}

// The pre-call logs LiteLLM's input and additional_args per handler, list_runs keeping the query under `params`
#[rstest]
#[case::create(Handler::CreateRun, "my run", Some(CREATE_BODY), None)]
#[case::list(Handler::ListRuns, "", None, Some(LIST_QUERY))]
#[case::get(Handler::GetRun, "", None, None)]
#[case::cancel(Handler::CancelRun, "", None, None)]
#[case::delete(Handler::DeleteRun, "", None, None)]
#[tokio::test]
async fn pre_call_records_input_and_additional_args(
    #[case] handler: Handler,
    #[case] input: &str,
    #[case] complete: Option<&str>,
    #[case] params: Option<&str>,
) {
    let expected = expectation(handler);
    let serving = json_server("200 OK", expected.reply, Duration::ZERO);
    let mut bench = Bench::new(
        url(&serving, handler_url(handler)),
        TransportLimits::DEFAULT,
    );
    let body = object(CREATE_BODY);
    let query = object(LIST_QUERY);
    run(&mut bench, call(handler, &body, &query, Some(TIMEOUT)))
        .await
        .expect("the fixture answers every handler");
    serving.finish().expect("the fixture finishes");
    assert_eq!(bench.fixture.input(), Some(&Json::Text(input.to_owned())));
    assert_eq!(
        bench.fixture.logged("api_base"),
        Some(&Json::Text(bench.url.clone()))
    );
    assert_eq!(
        bench.fixture.logged("headers"),
        Some(&json(r#"{"x-test":"yes"}"#))
    );
    assert_eq!(
        bench.fixture.logged("complete_input_dict"),
        complete.map(json).as_ref()
    );
    assert_eq!(bench.fixture.logged("params"), params.map(json).as_ref());
    let snapshot = bench.fixture.snapshot().expect("the logger saw the call");
    let Json::Object(fields) = snapshot else {
        panic!("the callback snapshot is an object");
    };
    assert_eq!(fields.get("input"), Some(&Json::Text(input.to_owned())));
    let Json::Object(args) = fields.get("additional_args").expect("the extra args") else {
        panic!("the extra args are an object");
    };
    assert_eq!(args.get("api_base"), Some(&Json::Text(bench.url.clone())));
    assert_eq!(args.get("complete_input_dict"), complete.map(json).as_ref());
    assert_eq!(args.get("params"), params.map(json).as_ref());
}

// A create body without a name logs an empty input, as LiteLLM's get default does
#[tokio::test]
async fn create_without_a_name_logs_an_empty_input() {
    let serving = json_server("200 OK", RUN, Duration::ZERO);
    let mut bench = Bench::new(url(&serving, CREATE_URL), TransportLimits::DEFAULT);
    let body = object(r#"{"data_source":{}}"#);
    run(
        &mut bench,
        Call::Create {
            timeout: Some(TIMEOUT),
            body: &body,
        },
    )
    .await
    .expect("the handler answers");
    serving.finish().expect("the fixture finishes");
    assert_eq!(bench.fixture.input(), Some(&Json::Text(String::new())));
}

// A pre-call that stops the call stops it before the send, so the upstream sees no request at all
#[tokio::test]
async fn pre_call_failure_stops_the_send() {
    let serving = json_server("200 OK", RUN, Duration::ZERO);
    let mut bench = Bench::new(url(&serving, CREATE_URL), TransportLimits::DEFAULT);
    bench.fixture.state.request_logging.raw_request_only = true;
    let body = object(CREATE_BODY);
    let error = run(
        &mut bench,
        Call::Create {
            timeout: Some(TIMEOUT),
            body: &body,
        },
    )
    .await
    .expect_err("a captured raw request stops the call");
    assert!(matches!(
        error,
        EvalHttpError::Logging(PrePostError::RawRequestCaptured)
    ));
    assert_no_request(serving);
}
