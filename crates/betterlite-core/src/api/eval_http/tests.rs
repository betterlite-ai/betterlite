#![cfg(test)]

mod support;

use std::time::Duration;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http_fixture::server::serve;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::evals::error::EvalsError;
use betterlite_test_ports::refused_port::RefusedPort;
use rstest::rstest;

use self::support::{
    Answer, Bench, Call, IMPATIENT, assert_line, assert_no_request, body_json, headers, json,
    json_server, object, provider, run, url,
};
use crate::api::eval_http::error::EvalHttpError;
use crate::api::eval_http::exchange::{EvalExchange, EvalMethod};

const TIMEOUT: Duration = Duration::from_millis(50);

const EVAL: &str = r#"{"id":"eval_1","object":"eval","created_at":1700000000,"updated_at":null,"name":"ev","data_source_config":{},"testing_criteria":[],"metadata":null}"#;
const PAGE: &str = r#"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#;
const DELETED: &str = r#"{"eval_id":"eval_1","object":"eval.deleted","deleted":true}"#;
const CANCELLED: &str = r#"{"id":"eval_1","object":"eval","status":"cancelled"}"#;
const CREATE_BODY: &str = r#"{"name":"my eval","data_source_config":{"type":"custom","item_schema":{}},"testing_criteria":[]}"#;
const UPDATE_BODY: &str = r#"{"name":"renamed","metadata":{"team":"evals"}}"#;
const LIST_QUERY: &str = r#"{"limit":5,"order":"asc"}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handler {
    Create,
    List,
    Get,
    Update,
    Delete,
    Cancel,
}

// What one handler puts on the wire and what reads its answer: the request line and body, the reply body, and the transform name
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
        Handler::Create => Expectation { method: "POST", path: "/v1/evals/eval_1", body: Some(CREATE_BODY), reply: EVAL, transform: "create_response" },
        Handler::List => Expectation { method: "GET", path: "/v1/evals/eval_1?limit=5&order=asc", body: None, reply: PAGE, transform: "list_response" },
        Handler::Get => Expectation { method: "GET", path: "/v1/evals/eval_1", body: None, reply: EVAL, transform: "get_response" },
        Handler::Update => Expectation { method: "POST", path: "/v1/evals/eval_1", body: Some(UPDATE_BODY), reply: EVAL, transform: "update_response" },
        Handler::Delete => Expectation { method: "DELETE", path: "/v1/evals/eval_1", body: None, reply: DELETED, transform: "delete_response" },
        Handler::Cancel => Expectation { method: "POST", path: "/v1/evals/eval_1", body: Some("{}"), reply: CANCELLED, transform: "cancel_response" },
    }
}

// The default call a handler's case runs, with the body or query that handler's request carries
fn call(handler: Handler) -> Call<'static> {
    let timeout = Some(TIMEOUT);
    match handler {
        Handler::Create => Call::Create {
            timeout,
            body: object(CREATE_BODY),
        },
        Handler::List => Call::List {
            query: object(LIST_QUERY),
        },
        Handler::Get => Call::Get,
        Handler::Update => Call::Update {
            timeout,
            body: object(UPDATE_BODY),
        },
        Handler::Delete => Call::Delete { timeout },
        Handler::Cancel => Call::Cancel { timeout },
    }
}

// The method, the path, the query and the body each handler puts on the wire, and the transform that reads the reply
#[rstest]
#[case::create(Handler::Create)]
#[case::list(Handler::List)]
#[case::get(Handler::Get)]
#[case::update(Handler::Update)]
#[case::delete(Handler::Delete)]
#[case::cancel(Handler::Cancel)]
#[tokio::test]
async fn handler_sends_litellm_method_and_body(#[case] handler: Handler) {
    let expected = expectation(handler);
    let serving = json_server("200 OK", expected.reply, Duration::ZERO);
    let mut bench = Bench::new(url(&serving, "/v1/evals/eval_1"), TransportLimits::DEFAULT);
    match run(&mut bench, call(handler))
        .await
        .expect("the handler answers")
    {
        Answer::Eval(eval) => assert_eq!(eval.id, "eval_1"),
        Answer::Page(page) => assert!(page.data.is_empty()),
        Answer::Deleted(deleted) => assert!(deleted.deleted),
        Answer::Cancelled(cancelled) => assert_eq!(cancelled.id, "eval_1"),
        Answer::Reply(reply) => {
            panic!("a handler transform never answers with the raw reply: {reply:?}")
        }
    }
    let request = serving.finish().expect("the fixture finishes");
    assert_line(&request, expected.method, expected.path);
    match expected.body {
        Some(body) => assert_eq!(body_json(&request), json(body)),
        None => assert!(request.body.is_empty()),
    }
    assert!(request.head.contains("x-test: yes"));
    assert_eq!(
        bench.events.recorded(),
        ["log", expected.transform].map(ToOwned::to_owned)
    );
    assert!(bench.config.asked().is_empty());
    let seen = bench.config.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].transform, expected.transform);
    assert_eq!(seen[0].status_code, 200);
    assert_eq!(seen[0].text, expected.reply);
    assert_eq!(
        seen[0].headers.get("x-upstream").map(String::as_str),
        Some("yes")
    );
}

// The caller's timeout reaches each send; an async post turns its timeout into litellm.Timeout, while the async delete re-raises it as a 500
#[rstest]
#[case::create(
    Handler::Create,
    408_u16,
    false,
    "litellm.Timeout: Connection timed out. Timeout passed=0.05, time taken="
)]
#[case::update(
    Handler::Update,
    408_u16,
    false,
    "litellm.Timeout: Connection timed out. Timeout passed=0.05, time taken="
)]
#[case::cancel(
    Handler::Cancel,
    408_u16,
    false,
    "litellm.Timeout: Connection timed out. Timeout passed=0.05, time taken="
)]
#[case::delete(
    Handler::Delete,
    500_u16,
    true,
    "the request ran past its read timeout or total deadline"
)]
#[tokio::test]
async fn caller_timeout_reaches_the_send_and_converts_per_method(
    #[case] handler: Handler,
    #[case] code: u16,
    #[case] synthesized: bool,
    #[case] message: &str,
) {
    let serving = json_server(
        "200 OK",
        expectation(handler).reply,
        Duration::from_millis(300),
    );
    let mut bench = Bench::new(url(&serving, "/v1/evals/eval_1"), TransportLimits::DEFAULT);
    let error = run(&mut bench, call(handler))
        .await
        .expect_err("the 50 ms timeout ends the call");
    let request = serving
        .finish()
        .expect("the fixture read the request before its pause");
    assert_line(
        &request,
        expectation(handler).method,
        expectation(handler).path,
    );
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
    assert_eq!(bench.config.asked().len(), 1);
}

// list and get take no timeout, so the client's read timeout is the deadline, and a deadline on a get is a synthesized 500
#[rstest]
#[case::list(Handler::List)]
#[case::get(Handler::Get)]
#[tokio::test]
async fn list_and_get_keep_the_clients_deadline(#[case] handler: Handler) {
    let serving = json_server(
        "200 OK",
        expectation(handler).reply,
        Duration::from_millis(300),
    );
    let mut bench = Bench::new(url(&serving, "/v1/evals/eval_1"), IMPATIENT);
    let error = run(&mut bench, call(handler))
        .await
        .expect_err("the client's 100 ms deadline ends the call");
    let request = serving
        .finish()
        .expect("the fixture read the request before its pause");
    assert_line(
        &request,
        expectation(handler).method,
        expectation(handler).path,
    );
    let provider = provider(&error);
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(
        provider.message,
        "the request ran past its read timeout or total deadline"
    );
    assert_eq!(bench.config.asked().len(), 1);
}

// A post 404, a delete 500, the boundary delete 400 and a 302 with no Location are status errors the client raises, so _handle_error carries the status and text
#[rstest]
#[case::create(
    Handler::Create,
    "404 Not Found",
    r#"{"error":{"message":"no such eval"}}"#,
    404_u16
)]
#[case::delete(
    Handler::Delete,
    "500 Internal Server Error",
    "the store is down",
    500_u16
)]
#[case::delete_bad_request(
    Handler::Delete,
    "400 Bad Request",
    r#"{"error":{"message":"bad request"}}"#,
    400_u16
)]
#[case::delete_unfollowed_redirect(
    Handler::Delete,
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
    let serving = json_server(status, body, Duration::ZERO);
    let mut bench = Bench::new(url(&serving, "/v1/evals/eval_1"), TransportLimits::DEFAULT);
    let error = run(&mut bench, call(handler))
        .await
        .expect_err("the upstream status ends the call");
    let request = serving.finish().expect("the fixture finishes");
    assert_line(
        &request,
        expectation(handler).method,
        expectation(handler).path,
    );
    let provider = provider(&error);
    assert_eq!(provider.status_code, code);
    assert!(!provider.status_code_is_synthesized);
    assert_eq!(provider.message, body);
    let upstream = provider
        .headers
        .as_ref()
        .expect("the upstream headers reach the error");
    assert!(upstream.iter().any(|(name, _)| name == "x-upstream"));
    let asked = bench.config.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].status_code, code);
    assert_eq!(asked[0].message, body);
    assert!(bench.config.seen().is_empty());
}

// HTTPHandler.get never raises for a status, so a get 404 reaches the transform, which cannot read the error body as an eval
#[tokio::test]
async fn get_status_error_reaches_the_transform() {
    let body = r#"{"error":{"message":"no such eval"}}"#;
    let serving = json_server("404 Not Found", body, Duration::ZERO);
    let mut bench = Bench::new(url(&serving, "/v1/evals/eval_1"), TransportLimits::DEFAULT);
    let error = run(&mut bench, Call::Get)
        .await
        .expect_err("the transform cannot read an error body as an eval");
    serving.finish().expect("the fixture finishes");
    assert!(matches!(
        error,
        EvalHttpError::Api(EvalsError::ReplyModel { .. })
    ));
    let seen = bench.config.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].transform, "get_response");
    assert_eq!(seen[0].status_code, 404);
    assert_eq!(seen[0].text, body);
    assert_eq!(
        seen[0].headers.get("x-upstream").map(String::as_str),
        Some("yes")
    );
    assert!(bench.config.asked().is_empty());
}

// A body that is not UTF-8 has no text form, so the transform is handed the same note httpx's decode failure stands for
#[tokio::test]
async fn get_binary_body_reaches_the_transform_as_its_size() {
    let body = [0xff_u8, 0xfe, 0x00];
    let mut answer = Vec::from("HTTP/1.1 200 OK\r\ncontent-length: 3\r\nconnection: close\r\n\r\n");
    answer.extend_from_slice(&body);
    let serving = serve(answer, Duration::ZERO).expect("the fixture serves");
    let mut bench = Bench::new(url(&serving, "/v1/evals/eval_1"), TransportLimits::DEFAULT);
    let error = run(&mut bench, Call::Get)
        .await
        .expect_err("the reply text is not JSON");
    serving.finish().expect("the fixture finishes");
    assert!(matches!(
        error,
        EvalHttpError::Api(EvalsError::ReplyJson { .. })
    ));
    let seen = bench.config.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].text, "3 bytes that are not UTF-8");
}

// A refused connection was never an HTTP answer, so _handle_error marks its 500 as synthesized
#[tokio::test]
async fn connection_refused_is_a_synthesized_500() {
    let refused = RefusedPort::bind().unwrap();
    let mut bench = Bench::new(
        format!("http://{}/v1/evals", refused.address),
        TransportLimits::DEFAULT,
    );
    let error = run(
        &mut bench,
        Call::Create {
            timeout: Some(TIMEOUT),
            body: object(CREATE_BODY),
        },
    )
    .await
    .expect_err("no listener answers");
    let provider = provider(&error);
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(provider.message, "the connection could not be made");
    let asked = bench.config.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].status_code, 500);
    assert_eq!(asked[0].message, "the connection could not be made");
}

// The pre-call logs LiteLLM's input and additional_args per handler, and the logger function the callbacks use sees them too
#[rstest]
#[case::create(Handler::Create, "my eval", Some(CREATE_BODY))]
#[case::list(Handler::List, "", Some(LIST_QUERY))]
#[case::get(Handler::Get, "", None)]
#[case::update(Handler::Update, "", Some(UPDATE_BODY))]
#[case::delete(Handler::Delete, "", None)]
#[case::cancel(Handler::Cancel, "", None)]
#[tokio::test]
async fn pre_call_records_input_and_additional_args(
    #[case] handler: Handler,
    #[case] input: &str,
    #[case] complete: Option<&str>,
) {
    let serving = json_server("200 OK", expectation(handler).reply, Duration::ZERO);
    let mut bench = Bench::new(url(&serving, "/v1/evals/eval_1"), TransportLimits::DEFAULT);
    run(&mut bench, call(handler))
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
}

// A pre-call that stops the call stops it before the send, so the upstream sees no request at all
#[tokio::test]
async fn pre_call_failure_stops_the_send() {
    let serving = json_server("200 OK", EVAL, Duration::ZERO);
    let mut bench = Bench::new(url(&serving, "/v1/evals/eval_1"), TransportLimits::DEFAULT);
    bench.fixture.state.request_logging.raw_request_only = true;
    let error = run(
        &mut bench,
        Call::Create {
            timeout: Some(TIMEOUT),
            body: object(CREATE_BODY),
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

#[derive(Debug, Clone, Copy)]
enum WireCase {
    Url,
    HeaderName,
    HeaderValue,
    QueryObject,
    QueryNestedList,
}

// A request the client cannot build fails inside the try block, so it reaches _handle_error as a synthesized 500 and never goes out
#[rstest]
#[case::bad_url(WireCase::Url, "the request URL not a url is not a URL")]
#[case::bad_header_name(WireCase::HeaderName, "the bad name header name is not a header name")]
#[case::bad_header_value(WireCase::HeaderValue, "the x-test header value is not a header value")]
#[case::object_query(WireCase::QueryObject, "the limit query value is not a scalar")]
#[case::nested_query(WireCase::QueryNestedList, "the limit query value is not a scalar")]
#[tokio::test]
async fn wire_failures_are_synthesized_500(#[case] case: WireCase, #[case] message: &str) {
    let serving = json_server("200 OK", EVAL, Duration::ZERO);
    let mut bench = match case {
        WireCase::Url => Bench::new(String::from("not a url"), TransportLimits::DEFAULT),
        WireCase::HeaderName => {
            let mut bench = Bench::new(url(&serving, "/v1/evals/eval_1"), TransportLimits::DEFAULT);
            bench.headers = headers(&[("bad name", "value")]);
            bench
        }
        WireCase::HeaderValue => {
            let mut bench = Bench::new(url(&serving, "/v1/evals/eval_1"), TransportLimits::DEFAULT);
            bench.headers = headers(&[("x-test", "bad\nvalue")]);
            bench
        }
        WireCase::QueryObject | WireCase::QueryNestedList => {
            Bench::new(url(&serving, "/v1/evals/eval_1"), TransportLimits::DEFAULT)
        }
    };
    let pending = match case {
        WireCase::Url | WireCase::HeaderName | WireCase::HeaderValue => Call::Create {
            timeout: Some(TIMEOUT),
            body: object(CREATE_BODY),
        },
        WireCase::QueryObject => Call::List {
            query: object(r#"{"limit":{"nested":1}}"#),
        },
        WireCase::QueryNestedList => Call::List {
            query: object(r#"{"limit":[[1]]}"#),
        },
    };
    let error = run(&mut bench, pending)
        .await
        .expect_err("the wire refuses the request");
    assert_no_request(serving);
    let provider = provider(&error);
    assert_eq!(provider.status_code, 500);
    assert!(provider.status_code_is_synthesized);
    assert_eq!(provider.message, message);
}

fn query_pairs_of(line: &str) -> Vec<&str> {
    let target = line.split_whitespace().nth(1).expect("the request target");
    let (_, query) = target.split_once('?').expect("the target carries a query");
    query.split('&').collect()
}

// httpx sends a list as repeated keys and a null as an empty value
#[tokio::test]
async fn query_values_render_like_httpx() {
    let serving = json_server("200 OK", PAGE, Duration::ZERO);
    let mut bench = Bench::new(url(&serving, "/v1/evals"), TransportLimits::DEFAULT);
    let query = object(r#"{"flag":true,"limit":[1,2],"missing":null,"order":"desc"}"#);
    run(&mut bench, Call::List { query })
        .await
        .expect("the list handler answers");
    let request = serving.finish().expect("the fixture finishes");
    assert!(
        request.line.starts_with("GET /v1/evals?"),
        "{}",
        request.line
    );
    let mut pairs = query_pairs_of(&request.line);
    pairs.sort_unstable();
    assert_eq!(
        pairs,
        vec!["flag=true", "limit=1", "limit=2", "missing=", "order=desc"]
    );
}

// eval_exchange is the public step B9371 reuses: it takes a body, a query, a timeout and the logged fields
#[tokio::test]
async fn eval_exchange_posts_a_body_and_a_query_for_reuse() {
    let serving = json_server("200 OK", "{}", Duration::ZERO);
    let mut bench = Bench::new(url(&serving, "/v1/evals/run"), TransportLimits::DEFAULT);
    let body = object(r#"{"name":"run"}"#);
    let query = object(r#"{"limit":1}"#);
    let exchange = EvalExchange {
        method: EvalMethod::Post,
        body: Some(&body),
        query: Some(&query),
        params: None,
        timeout: Some(TIMEOUT),
        input: "run",
        complete_input_dict: Some(&body),
    };
    let answered = run(&mut bench, Call::Exchange(&exchange))
        .await
        .expect("the exchange answers");
    let Answer::Reply(reply) = answered else {
        panic!("the exchange answers with the raw reply");
    };
    assert_eq!(reply.status_code, 200);
    assert_eq!(reply.text, "{}");
    let request = serving.finish().expect("the fixture finishes");
    assert_line(&request, "POST", "/v1/evals/run?limit=1");
    assert_eq!(body_json(&request), json(r#"{"name":"run"}"#));
    assert!(bench.config.seen().is_empty());
    assert_eq!(bench.fixture.input(), Some(&Json::Text("run".to_owned())));
}
