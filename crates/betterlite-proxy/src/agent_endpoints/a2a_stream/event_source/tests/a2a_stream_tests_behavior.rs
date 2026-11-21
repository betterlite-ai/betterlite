use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use betterlite_http::proxy::ProxiedClient;
use futures_util::StreamExt as _;
use futures_util::stream::{iter, once, pending};
use rstest::rstest;
use tokio::time::{sleep, timeout};

use crate::agent_endpoints::a2a_stream::error::StreamError;
use crate::agent_endpoints::a2a_stream::event_source::{
    EventSourceRequest, a2a_sse_event_source, data_events,
};
use crate::agent_endpoints::a2a_stream::tests::fakes::{
    Agent, CLOSED, CountingConverter, Reply, client, client_with_body_limit, json, serve_agent,
};

const NO_HEADERS: [(&str, &str); 0] = [];
const NOTHING: [&str; 0] = [];

async fn agent(status: u16, headers: &'static str, parts: Vec<&'static [u8]>) -> Agent {
    serve_agent(Reply {
        status,
        headers,
        parts,
        hold_open: false,
    })
    .await
}

fn request(url: &str, extra_headers: &[(&str, &str)]) -> EventSourceRequest {
    EventSourceRequest {
        agent_url: url.to_owned(),
        body: json(
            r#"{"jsonrpc": "2.0", "id": "r1", "method": "tasks/resubscribe", "params": {"id": "t1"}}"#,
        ),
        request_id: Some(AgentJson::from("r1")),
        extra_headers: extra_headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

async fn events(
    client: &ProxiedClient,
    source: EventSourceRequest,
) -> Vec<Result<AgentJson, StreamError>> {
    a2a_sse_event_source(client, &CountingConverter, source)
        .collect()
        .await
}

#[rstest]
#[case::defaults(&NO_HEADERS, &["content-type: application/json", "accept: text/event-stream"], &NOTHING)]
#[case::extra_header(&[("X-Trace", "t1")], &["x-trace: t1", "accept: text/event-stream"], &NOTHING)]
#[case::extra_accept_wins(&[("accept", "application/json")], &["accept: application/json"], &["text/event-stream"])]
#[case::extra_content_type_wins(&[("Content-Type", "application/a2a+json")], &["content-type: application/a2a+json"], &["content-type: application/json"])]
#[case::later_extra_wins(&[("x-a", "1"), ("X-A", "2")], &["x-a: 2"], &["x-a: 1"])]
#[tokio::test]
async fn posts_the_body_with_its_headers(
    #[case] extra_headers: &[(&str, &str)],
    #[case] present: &[&str],
    #[case] absent: &[&str],
) {
    let server = agent(200, "", vec![b"data: {}\n\n"]).await;
    let url = format!("{}/a2a/agent-1", server.url);
    let received = events(&client(), request(&url, extra_headers)).await;
    assert_eq!(received.len(), 1);
    let sent = server.requests.lock().unwrap().join("");
    assert!(sent.starts_with("POST /a2a/agent-1 HTTP/1.1\r\n"), "{sent}");
    let (head, body) = sent.split_once("\r\n\r\n").unwrap();
    let lines: Vec<&str> = head.lines().collect();
    assert!(present.iter().all(|line| lines.contains(line)), "{sent}");
    assert!(absent.iter().all(|text| !head.contains(text)), "{sent}");
    assert_eq!(
        serde_json::from_str::<AgentJson>(body).unwrap(),
        request(&url, &[]).body
    );
}

#[tokio::test]
async fn yields_one_event_per_data_line_in_order() {
    let server = agent(
        200,
        "",
        vec![
            b"data: {\"a\": 1}\n\nevent: update\nid: 3\n: note\nretry: 5\ndata: [1, 2]\r\n",
            b"data: 7\rdata: not json\ndata:\n  data: {\"b\": 2}\r",
            b"\ndata: \"s\"\ndata: {\"c\": 3}",
        ],
    )
    .await;
    let received: Vec<AgentJson> = events(&client(), request(&server.url, &[]))
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        received,
        vec![
            json(r#"{"a": 1, "conversions": 1, "request_id": "r1"}"#),
            json("[1, 2]"),
            AgentJson::from(7_i32),
            json(r#"{"b": 2, "conversions": 1, "request_id": "r1"}"#),
            AgentJson::from("s"),
            json(r#"{"c": 3, "conversions": 1, "request_id": "r1"}"#),
        ]
    );
}

const NOT_FOUND: &str =
    r#"{"jsonrpc": "2.0", "id": "r1", "error": {"code": -32603, "message": "Not Found"}}"#;

#[rstest]
#[case::jsonrpc_error_gets_the_request_id(
    404,
    r#"{"jsonrpc": "2.0", "error": {"code": -32001, "message": "Task not found"}}"#,
    Some("r1"),
    r#"{"jsonrpc": "2.0", "id": "r1", "error": {"code": -32001, "message": "Task not found"}}"#
)]
#[case::jsonrpc_error_keeps_its_id(
    400,
    r#"{"id": "u1", "error": {"code": -32600}}"#,
    Some("r1"),
    r#"{"id": "u1", "error": {"code": -32600}}"#
)]
#[case::null_error(
    500,
    r#"{"error": null}"#,
    Some("r1"),
    r#"{"error": null, "id": "r1"}"#
)]
#[case::error_without_request_id(
    404,
    r#"{"error": {"code": 1}}"#,
    None,
    r#"{"error": {"code": 1}}"#
)]
#[case::object_without_error(404, r#"{"detail": "missing"}"#, Some("r1"), NOT_FOUND)]
#[case::list_body(404, r#"[{"error": 1}]"#, Some("r1"), NOT_FOUND)]
#[case::sse_body(500, "data: {\"a\": 1}\n\n", Some("r1"), r#"{"jsonrpc": "2.0", "id": "r1", "error": {"code": -32603, "message": "Internal Server Error"}}"#)]
#[case::fallback_without_request_id(
    400,
    "",
    None,
    r#"{"jsonrpc": "2.0", "id": null, "error": {"code": -32603, "message": "Bad Request"}}"#
)]
#[case::no_canonical_reason(
    599,
    "",
    Some("r1"),
    r#"{"jsonrpc": "2.0", "id": "r1", "error": {"code": -32603, "message": ""}}"#
)]
#[case::redirect(
    302,
    "",
    Some("r1"),
    r#"{"jsonrpc": "2.0", "id": "r1", "error": {"code": -32603, "message": "Found"}}"#
)]
#[case::last_success_status(
    299,
    "data: {\"a\": 1}\n\n",
    Some("r1"),
    r#"{"a": 1, "conversions": 1, "request_id": "r1"}"#
)]
#[case::first_error_status(
    300,
    "",
    Some("r1"),
    r#"{"jsonrpc": "2.0", "id": "r1", "error": {"code": -32603, "message": "Multiple Choices"}}"#
)]
#[tokio::test]
async fn an_error_status_is_one_unconverted_event(
    #[case] status: u16,
    #[case] body: &'static str,
    #[case] request_id: Option<&str>,
    #[case] event: &str,
) {
    let server = agent(status, "", vec![body.as_bytes()]).await;
    let mut source = request(&server.url, &[]);
    source.request_id = request_id.map(AgentJson::from);
    let received: Vec<AgentJson> = events(&client(), source)
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        received,
        vec![serde_json::from_str::<AgentJson>(event).unwrap()]
    );
}

#[rstest]
#[case::refused(CLOSED, &NO_HEADERS, "the A2A agent could not be reached")]
#[case::not_a_url("not a url", &NO_HEADERS, "the A2A agent URL is not a valid URL")]
#[case::bad_header_name(CLOSED, &[("bad header", "1")], "the extra header name \"bad header\" is not a valid header name")]
#[case::bad_header_value(CLOSED, &[("x-a", "a\nb")], "the extra header x-a holds a character a header cannot carry")]
#[tokio::test]
async fn a_request_failure_is_one_error_item(
    #[case] url: &str,
    #[case] extra_headers: &[(&str, &str)],
    #[case] message: &str,
) {
    let received = events(&client(), request(url, extra_headers)).await;
    let failures: Vec<String> = received
        .into_iter()
        .map(|item| item.unwrap_err().to_string())
        .collect();
    assert_eq!(failures, [message]);
}

#[tokio::test]
async fn a_broken_body_ends_with_an_error_item() {
    let server = agent(200, "content-length: 100\r\n", vec![b"data: 1\n"]).await;
    let mut received = events(&client(), request(&server.url, &[])).await;
    assert!(matches!(received.pop(), Some(Err(StreamError::Body(_)))));
    assert_eq!(
        received.into_iter().map(Result::unwrap).collect::<Vec<_>>(),
        [AgentJson::from(1_i32)]
    );
}

#[rstest]
#[case::at_the_limit(b"data: 1234567890", Ok(1_234_567_890))]
#[case::past_the_limit(
    b"data: 12345678901",
    Err("the A2A agent sent a line longer than 16 bytes")
)]
#[tokio::test]
async fn a_line_longer_than_the_body_limit_ends_the_stream(
    #[case] part: &'static [u8],
    #[case] outcome: Result<u64, &str>,
) {
    let server = agent(200, "", vec![part]).await;
    let received = events(&client_with_body_limit(16), request(&server.url, &[])).await;
    let observed: Vec<Result<u64, String>> = received
        .into_iter()
        .map(|item| {
            item.map(|event| event.as_u64().unwrap())
                .map_err(|error| error.to_string())
        })
        .collect();
    assert_eq!(observed, [outcome.map_err(str::to_owned)]);
}

#[tokio::test]
async fn dropping_the_stream_closes_the_upstream_connection() {
    let server = serve_agent(Reply {
        status: 200,
        headers: "",
        parts: vec![b"data: 1\n\n"],
        hold_open: true,
    })
    .await;
    let mut source = a2a_sse_event_source(&client(), &CountingConverter, request(&server.url, &[]));
    assert_eq!(
        source.next().await.unwrap().unwrap(),
        AgentJson::from(1_i32)
    );
    drop(source);
    timeout(Duration::from_secs(5), server.closed.notified())
        .await
        .unwrap();
}

#[tokio::test]
async fn follows_redirects_like_the_litellm_client() {
    let server = agent(307, "location: /a2a\r\n", Vec::new()).await;
    let received = events(&client(), request(&server.url, &[])).await;
    let failures: Vec<String> = received
        .into_iter()
        .map(|item| item.unwrap_err().to_string())
        .collect();
    assert_eq!(failures, ["the A2A agent could not be reached"]);
    assert_eq!(server.requests.lock().unwrap().len(), 11);
}

#[rstest]
#[case::chunk_just_in_time(599, Ok(2))]
#[case::agent_silent_too_long(601, Err("the A2A agent sent nothing for 600 seconds"))]
#[tokio::test(start_paused = true)]
async fn a_silent_agent_ends_the_stream_after_the_read_timeout(
    #[case] silence: u64,
    #[case] outcome: Result<u64, &str>,
) {
    let late = once(async move {
        sleep(Duration::from_secs(silence)).await;
        Ok(b"data: 2\n".to_vec())
    });
    let body = iter([Ok(b"data: 1\n".to_vec())])
        .chain(late)
        .chain(pending())
        .boxed();
    let received: Vec<Result<u64, String>> = data_events(body, 64)
        .take(2)
        .map(|item| {
            item.map(|event| event.as_u64().unwrap())
                .map_err(|error| error.to_string())
        })
        .collect()
        .await;
    assert_eq!(received, [Ok(1), outcome.map_err(str::to_owned)]);
}
