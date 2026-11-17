#![cfg(test)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::straiker::delivery::StraikerDelivery;
use crate::straiker::delivery::attempts::{PayloadError, encode};
use crate::straiker::delivery::blocked_turns::BlockedTurns;
use crate::straiker::delivery::envelope::response_shape;
use crate::straiker::delivery::fixtures::{
    Clock, FakeDelay, Fakes, V1_ALLOW, V3_ALLOW, api_inputs, caps, delivery, inputs, network_error,
    object, permanent_error, raw, reply, request_data, run, sent, settings, v3_raw,
};
use crate::straiker::delivery::transport::WebhookHeaders;
use crate::straiker::delivery::transport::{PooledPost, WebhookPost as _, WebhookRequest};
use crate::straiker::v3::answer_shape::Response;

const FULL_INPUTS: &str = r#"{
    "texts": ["hi"],
    "images": ["img"],
    "structured_messages": [{"role": "user", "content": "hi"}],
    "tools": [{"type": "function", "function": {"name": "f"}}],
    "tool_calls": [{"id": "c1"}],
    "model": "gpt-4o"
}"#;

const REQUEST_DATA: &str = r#"{
    "model": "gpt-4o-fallback",
    "metadata": {
        "agent_id": "agent-7",
        "app_name": "app-1",
        "note": "memo",
        "user_api_key_user_id": "alice",
        "session_id": "sess-1"
    }
}"#;

const REQUEST_ENVELOPE: &str = r#"{
    "schema_version": "1",
    "event": {
        "type": "pre_call",
        "id": "call-1:request",
        "stream": {"phase": "none"}
    },
    "request": {
        "texts": ["hi"],
        "images": ["img"],
        "structured_messages": [{"role": "user", "content": "hi"}],
        "tools": [{"type": "function", "function": {"name": "f"}}],
        "tool_calls": [{"id": "c1"}]
    },
    "context": {
        "call_surface": "completion",
        "mode": ["pre_call"],
        "model": "gpt-4o",
        "session_id": "sess-1",
        "litellm_call_id": "call-1",
        "litellm_version": "1.105.1"
    },
    "identity": {"litellm_user_id": "alice"},
    "application": {"source": "agent-7", "name": "app-1"},
    "metadata": {"note": "memo"}
}"#;

const RESPONSE_INPUTS: &str = r#"{"texts": ["answer"]}"#;

const RESPONSE_DATA: &str = r#"{
    "model": "gpt-4o",
    "messages": [
        {"role": "user", "content": "hi"},
        {"role": "assistant", "content": "hello"}
    ],
    "response": {
        "choices": [{"finish_reason": "stop"}],
        "usage": {"prompt_tokens": 3, "completion_tokens": 4}
    },
    "stream": true
}"#;

const RESPONSE_ENVELOPE: &str = r#"{
    "schema_version": "1",
    "event": {
        "type": "post_call",
        "id": "call-2:response",
        "stream": {"phase": "assembled"}
    },
    "request": {
        "texts": [],
        "images": [],
        "structured_messages": [
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "hello"}
        ]
    },
    "response": {
        "texts": ["answer"],
        "images": [],
        "finish_reason": "stop"
    },
    "context": {
        "call_surface": "completion",
        "mode": ["pre_call"],
        "model": "gpt-4o",
        "litellm_call_id": "call-2",
        "litellm_version": "1.105.1"
    },
    "identity": {},
    "application": {"source": "LiteLLM Gateway"},
    "usage": {"input_tokens": 3, "output_tokens": 4}
}"#;

const SIMPLE_DATA: &str = r#"{"model": "gpt-4o"}"#;
const BODY: &str = r#"{"texts":["hi"]}"#;

#[tokio::test]
async fn a_request_phase_posts_the_v1_envelope() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_ALLOW)]);
    let mut data = request_data(REQUEST_DATA);
    let call = fakes.call(Some("completion"), Some("call-1"));
    let answer = run(
        &handler,
        &mut data,
        api_inputs(FULL_INPUTS),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    assert_eq!(answer, api_inputs(FULL_INPUTS));
    let posts = sent(&handler);
    assert_eq!(
        object(&posts.first().unwrap().body),
        object(REQUEST_ENVELOPE)
    );
}

#[tokio::test]
async fn a_response_phase_posts_the_answer_beside_its_request() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_ALLOW)]);
    let mut data = request_data(RESPONSE_DATA);
    let call = fakes.call(Some("completion"), Some("call-2"));
    run(
        &handler,
        &mut data,
        api_inputs(RESPONSE_INPUTS),
        GuardrailInputType::Response,
        &call,
    )
    .await
    .expect("an allowed answer");
    let posts = sent(&handler);
    assert_eq!(
        object(&posts.first().unwrap().body),
        object(RESPONSE_ENVELOPE)
    );
}

#[tokio::test]
async fn a_response_phase_without_streaming_reports_no_phase() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_ALLOW)]);
    let mut data = request_data(
        r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "response": {"choices": [{"finish_reason": "stop"}]}
        }"#,
    );
    let call = fakes.call(Some("completion"), Some("call-3"));
    run(
        &handler,
        &mut data,
        api_inputs(RESPONSE_INPUTS),
        GuardrailInputType::Response,
        &call,
    )
    .await
    .expect("an allowed answer");
    let posts = sent(&handler);
    let body = object(&posts.first().unwrap().body);
    let event = body.get("event").and_then(Json::as_object).unwrap();
    let stream = event.get("stream").and_then(Json::as_object).unwrap();
    assert_eq!(stream.get("phase"), Some(&Json::Text("none".to_owned())));
}

#[tokio::test]
async fn the_event_id_falls_back_to_litellm() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_ALLOW)]);
    let mut data = request_data(SIMPLE_DATA);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    let posts = sent(&handler);
    let body = object(&posts.first().unwrap().body);
    let event = body.get("event").and_then(Json::as_object).unwrap();
    assert_eq!(
        event.get("id"),
        Some(&Json::Text("litellm:request".to_owned()))
    );
}

async fn delivered_size() -> usize {
    let fakes = Fakes::default();
    let call = fakes.call(Some("completion"), Some("call-1"));
    let handler = delivery(settings(&raw()), vec![reply(200, V1_ALLOW)]);
    let mut data = request_data(SIMPLE_DATA);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("the first delivery");
    sent(&handler).first().unwrap().body.len()
}

#[tokio::test]
async fn a_payload_at_the_limit_is_delivered() {
    let fakes = Fakes::default();
    let call = fakes.call(Some("completion"), Some("call-1"));
    let size = delivered_size().await;
    let mut args = raw();
    args.max_payload_bytes = Some(i64::try_from(size).unwrap());
    let handler = delivery(settings(&args), vec![reply(200, V1_ALLOW)]);
    let mut data = request_data(SIMPLE_DATA);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("a payload at the limit is sent");
    assert_eq!(sent(&handler).len(), 1);
}

#[tokio::test]
async fn a_payload_one_byte_over_the_limit_is_a_delivery_error() {
    let fakes = Fakes::default();
    let call = fakes.call(Some("completion"), Some("call-1"));
    let size = delivered_size().await;
    let limit = size.saturating_sub(1);
    let mut args = raw();
    args.max_payload_bytes = Some(i64::try_from(limit).unwrap());
    let handler = delivery(settings(&args), vec![reply(200, V1_ALLOW)]);
    let mut data = request_data(SIMPLE_DATA);
    let failure = run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the payload is too large");
    assert!(sent(&handler).is_empty());
    let GuardrailFailure::GuardrailRaised {
        message,
        blocked_content,
        ..
    } = failure
    else {
        panic!("expected a block");
    };
    assert!(!blocked_content);
    assert_eq!(
        message,
        format!(
            "Straiker detection unavailable: payload {size}B exceeds max_payload_bytes {limit}"
        )
    );
}

#[test]
fn a_payload_that_cannot_be_serialized_is_a_delivery_error() {
    let map = BTreeMap::from([((1_i64, 2_i64), true)]);
    let error = encode(&map, 1024).expect_err("integer keys cannot be JSON keys");
    assert!(matches!(error, PayloadError::Serialize(_)));
    assert_eq!(
        error.to_string(),
        "request serialization failed: key must be a string"
    );
}

#[test]
fn an_oversized_payload_names_both_sizes() {
    let error = encode(&"abcdef", 5).expect_err("the payload is too large");
    assert!(matches!(
        error,
        PayloadError::TooLarge { bytes: 8, limit: 5 }
    ));
    assert_eq!(error.to_string(), "payload 8B exceeds max_payload_bytes 5");
}

#[tokio::test]
async fn a_value_that_is_not_json_is_a_permanent_error() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![reply(200, V1_ALLOW)]);
    let mut data = BTreeMap::from([("temperature".to_owned(), GuardrailValue::Decimal(f64::NAN))]);
    let call = fakes.call(Some("completion"), None);
    let failure = run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the value is not JSON");
    assert!(sent(&handler).is_empty());
    let GuardrailFailure::GuardrailRaised { message, .. } = failure else {
        panic!("expected a block");
    };
    assert_eq!(
        message,
        "Straiker detection unavailable: the value NaN is not a JSON number"
    );
}

#[tokio::test]
async fn a_transient_failure_is_retried_with_the_backoff() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&raw()),
        vec![network_error(), reply(200, V1_ALLOW)],
    );
    let mut data = request_data(SIMPLE_DATA);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("the second attempt is allowed");
    let posts = sent(&handler);
    assert_eq!(posts.len(), 2);
    assert_eq!(posts.first().unwrap().body, posts[1].body);
    assert_eq!(caps(&handler), vec![Duration::from_millis(100)]);
}

#[tokio::test]
async fn a_permanent_failure_is_not_retried() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&raw()), vec![permanent_error()]);
    let mut data = request_data(SIMPLE_DATA);
    let call = fakes.call(Some("completion"), None);
    let failure = run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("the failure is permanent");
    assert_eq!(sent(&handler).len(), 1);
    assert!(caps(&handler).is_empty());
    assert_eq!(
        failure.to_string(),
        "Straiker detection unavailable: decode error"
    );
}

#[tokio::test]
async fn the_attempt_limit_is_the_retry_count_plus_one() {
    let fakes = Fakes::default();
    let handler = delivery(
        settings(&raw()),
        vec![network_error(), network_error(), network_error()],
    );
    let mut data = request_data(SIMPLE_DATA);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect_err("every attempt fails");
    assert_eq!(sent(&handler).len(), 3);
    assert_eq!(
        caps(&handler),
        vec![Duration::from_millis(100), Duration::from_millis(200)]
    );
}

fn webhook_headers() -> WebhookHeaders {
    WebhookHeaders {
        authorization: SecretString::from("Bearer test-key"),
        values: BTreeMap::from([("Content-Type".to_owned(), "application/json".to_owned())]),
    }
}

fn pooled() -> PooledPost {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    PooledPost {
        client: ProxiedClient::outbound(&policy).unwrap(),
    }
}

fn request<'request>(
    url: &'request str,
    timeout: Duration,
    headers: &'request WebhookHeaders,
) -> WebhookRequest<'request> {
    WebhookRequest {
        url,
        body: BODY,
        headers,
        timeout,
    }
}

// A mutant that stops the client from connecting must fail its test, not leave this accepted forever.
const SERVE_TIMEOUT: Duration = Duration::from_secs(10);

async fn accept(listener: &TcpListener) -> (TcpStream, SocketAddr) {
    timeout(SERVE_TIMEOUT, listener.accept())
        .await
        .expect("the client connected")
        .unwrap()
}

async fn serve_one(listener: TcpListener, reply: &'static [u8]) -> Vec<u8> {
    let (mut stream, _) = accept(&listener).await;
    let mut received = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !received.ends_with(BODY.as_bytes()) {
        let read = stream.read(&mut chunk).await.unwrap();
        assert_ne!(read, 0, "the client closed before the body arrived");
        received.extend_from_slice(&chunk[..read]);
    }
    stream.write_all(reply).await.unwrap();
    received
}

#[tokio::test]
async fn a_pooled_post_sends_the_request_over_http() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let post = pooled();
    let headers = webhook_headers();
    let url = format!("http://{address}/api/v1/detect/webhook");
    let (answer, received) = tokio::join!(
        post.post(request(&url, Duration::from_secs(2), &headers)),
        serve_one(
            listener,
            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}"
        )
    );
    let response = answer.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body, "{}");
    let text = String::from_utf8(received).unwrap();
    assert!(text.starts_with("POST /api/v1/detect/webhook HTTP/1.1"));
    let lowercase = text.to_ascii_lowercase();
    assert!(lowercase.contains("authorization: bearer test-key"));
    assert!(lowercase.contains("content-type: application/json"));
    assert!(text.ends_with(BODY));
}

#[tokio::test]
async fn a_pooled_post_returns_a_non_200_status_unchanged() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let post = pooled();
    let headers = webhook_headers();
    let url = format!("http://{address}/api/v1/detect/webhook");
    let (answer, _received) = tokio::join!(
        post.post(request(&url, Duration::from_secs(2), &headers)),
        serve_one(
            listener,
            b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 6\r\nconnection: close\r\n\r\ndenied"
        )
    );
    let response = answer.unwrap();
    assert_eq!(response.status, 500);
    assert_eq!(response.body, "denied");
}

#[tokio::test]
async fn an_invalid_url_is_an_unreachable_failure() {
    let post = pooled();
    let headers = webhook_headers();
    let failure = post
        .post(request("not a url", Duration::from_secs(1), &headers))
        .await
        .expect_err("the URL is invalid");
    assert!(failure.is_unreachable);
    assert!(failure.retryable);
    assert!(
        failure.message.contains("not valid"),
        "got {}",
        failure.message
    );
}

#[rstest]
#[case::name("bad name", "value")]
#[case::value("X-Ok", "line\nbreak")]
#[tokio::test]
async fn an_unusable_header_is_an_unreachable_failure(#[case] name: &str, #[case] value: &str) {
    let post = pooled();
    let headers = WebhookHeaders {
        authorization: SecretString::from("Bearer test-key"),
        values: BTreeMap::from([(name.to_owned(), value.to_owned())]),
    };
    let failure = post
        .post(request(
            "http://127.0.0.1:9/x",
            Duration::from_secs(1),
            &headers,
        ))
        .await
        .expect_err("the header is invalid");
    assert!(failure.is_unreachable);
    assert!(failure.retryable);
    assert!(
        failure.message.contains("not valid"),
        "got {}",
        failure.message
    );
}

#[rstest]
#[case::name("X-\u{dc}nicode", "value")]
#[case::value("X-Ok", "v\u{e4}lue")]
#[tokio::test]
async fn a_non_ascii_header_is_a_permanent_failure(#[case] name: &str, #[case] value: &str) {
    let post = pooled();
    let headers = WebhookHeaders {
        authorization: SecretString::from("Bearer test-key"),
        values: BTreeMap::from([(name.to_owned(), value.to_owned())]),
    };
    let failure = post
        .post(request(
            "http://127.0.0.1:9/x",
            Duration::from_secs(1),
            &headers,
        ))
        .await
        .expect_err("the header is not ASCII");
    assert!(!failure.is_unreachable);
    assert!(!failure.retryable);
    assert!(
        failure.message.contains("not ASCII"),
        "got {}",
        failure.message
    );
}

#[tokio::test]
async fn a_non_ascii_authorization_is_a_permanent_failure() {
    let post = pooled();
    let headers = WebhookHeaders {
        authorization: SecretString::from("Bearer k\u{eb}y"),
        values: BTreeMap::new(),
    };
    let failure = post
        .post(request(
            "http://127.0.0.1:9/x",
            Duration::from_secs(1),
            &headers,
        ))
        .await
        .expect_err("the authorization is not ASCII");
    assert!(!failure.is_unreachable);
    assert!(!failure.retryable);
    assert_eq!(
        failure.message,
        "the Authorization header is not ASCII".to_owned()
    );
}

#[tokio::test]
async fn a_refused_connection_is_an_unreachable_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let post = pooled();
    let headers = webhook_headers();
    let url = format!("http://{address}/x");
    let failure = post
        .post(request(&url, Duration::from_secs(1), &headers))
        .await
        .expect_err("nothing listens there");
    assert!(failure.is_unreachable);
    assert!(failure.retryable);
    assert!(!failure.message.is_empty());
}

#[tokio::test]
async fn a_stalled_answer_is_an_unreachable_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut server = JoinSet::new();
    server.spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        sleep(Duration::from_secs(30)).await;
        drop(stream);
    });
    let post = pooled();
    let headers = webhook_headers();
    let url = format!("http://{address}/x");
    let failure = post
        .post(request(&url, Duration::from_millis(50), &headers))
        .await
        .expect_err("the answer never comes");
    server.abort_all();
    assert!(failure.is_unreachable);
    assert!(failure.retryable);
    assert!(!failure.message.is_empty());
}

#[tokio::test]
async fn a_pooled_url_error_retries_and_fails_open() {
    let fakes = Fakes::default();
    let mut args = raw();
    args.api_base = "not a url".to_owned();
    args.unreachable_fallback = "fail_open".to_owned();
    let handler = StraikerDelivery {
        settings: settings(&args),
        transport: pooled(),
        delay: FakeDelay::default(),
        blocked_turns: BlockedTurns::new(Clock::default().source()).unwrap(),
    };
    let mut data = request_data(SIMPLE_DATA);
    let call = fakes.call(Some("completion"), None);
    let answer = run(
        &handler,
        &mut data,
        inputs(&["hi"]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an unreachable endpoint fails open");
    assert_eq!(answer, inputs(&["hi"]));
    assert_eq!(caps(&handler).len(), 2);
}

// Answers `count` connections with `reply` once each request, headers and body, has arrived.
async fn serve_requests(listener: TcpListener, reply: &'static [u8], count: usize) -> Vec<String> {
    let mut requests = Vec::new();
    for _ in 0..count {
        let (mut stream, _) = accept(&listener).await;
        let mut received = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = stream.read(&mut chunk).await.unwrap();
            assert_ne!(read, 0, "the client closed before the request arrived");
            received.extend_from_slice(&chunk[..read]);
            let text = str::from_utf8(&received).unwrap().to_lowercase();
            let Some((head, body)) = text.split_once("\r\n\r\n") else {
                continue;
            };
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .map_or(0, |value| value.trim().parse::<usize>().unwrap());
            if body.len() >= length {
                break;
            }
        }
        stream.write_all(reply).await.unwrap();
        requests.push(str::from_utf8(&received).unwrap().to_owned());
    }
    requests
}

fn pooled_handler(
    api_base: &str,
    max_retries: i64,
    fallback: &str,
) -> StraikerDelivery<PooledPost, FakeDelay> {
    let mut args = raw();
    args.api_base = api_base.to_owned();
    args.max_retries = Some(max_retries);
    args.unreachable_fallback = fallback.to_owned();
    StraikerDelivery {
        settings: settings(&args),
        transport: pooled(),
        delay: FakeDelay::default(),
        blocked_turns: BlockedTurns::new(Clock::default().source()).unwrap(),
    }
}

#[tokio::test]
async fn a_non_utf8_ok_answer_is_a_permanent_schema_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let fakes = Fakes::default();
    let handler = pooled_handler(&format!("http://{address}"), 2, "fail_open");
    let mut data = request_data(SIMPLE_DATA);
    let call = fakes.call(Some("completion"), None);
    let (answer, _received) = tokio::join!(
        run(
            &handler,
            &mut data,
            inputs(&["hi"]),
            GuardrailInputType::Request,
            &call
        ),
        serve_requests(
            listener,
            b"HTTP/1.1 200 OK\r\ncontent-length: 1\r\nconnection: close\r\n\r\n\xff",
            1
        )
    );
    let failure = answer.expect_err("the body is not a verdict");
    assert!(caps(&handler).is_empty());
    let GuardrailFailure::GuardrailRaised {
        message,
        blocked_content,
        ..
    } = failure
    else {
        panic!("expected a block");
    };
    assert!(!blocked_content);
    assert!(message.contains("invalid response schema"), "got {message}");
}

#[tokio::test]
async fn a_non_utf8_error_page_is_retried_and_fails_open() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let fakes = Fakes::default();
    let handler = pooled_handler(&format!("http://{address}"), 1, "fail_open");
    let mut data = request_data(SIMPLE_DATA);
    let call = fakes.call(Some("completion"), None);
    let (answer, received) = tokio::join!(
        run(
            &handler,
            &mut data,
            inputs(&["hi"]),
            GuardrailInputType::Request,
            &call
        ),
        serve_requests(
            listener,
            b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 1\r\nconnection: close\r\n\r\n\xff",
            2
        )
    );
    assert_eq!(
        answer.expect("a 503 under fail_open lets the call pass"),
        inputs(&["hi"])
    );
    assert_eq!(received.len(), 2);
    assert_eq!(caps(&handler).len(), 1);
}

#[tokio::test]
async fn headers_that_differ_only_in_case_are_both_sent() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let post = pooled();
    let headers = WebhookHeaders {
        authorization: SecretString::from("Bearer test-key"),
        values: BTreeMap::from([
            ("X-Foo".to_owned(), "upper".to_owned()),
            ("x-foo".to_owned(), "lower".to_owned()),
        ]),
    };
    let url = format!("http://{address}/x");
    let (answer, received) = tokio::join!(
        post.post(request(&url, Duration::from_secs(2), &headers)),
        serve_requests(
            listener,
            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
            1
        )
    );
    answer.expect("the post succeeds");
    let text = received.first().unwrap().to_lowercase();
    assert!(text.contains("x-foo: upper\r\n"), "got {text}");
    assert!(text.contains("x-foo: lower\r\n"), "got {text}");
}

#[tokio::test]
async fn a_v3_request_posts_the_provider_body() {
    let fakes = Fakes::default();
    let handler = delivery(settings(&v3_raw()), vec![reply(200, V3_ALLOW)]);
    let mut data =
        request_data(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#);
    let call = fakes.call(Some("completion"), None);
    run(
        &handler,
        &mut data,
        inputs(&[]),
        GuardrailInputType::Request,
        &call,
    )
    .await
    .expect("an allowed call");
    let posts = sent(&handler);
    let body = object(&posts.first().unwrap().body);
    assert_eq!(body.get("model"), Some(&Json::Text("gpt-4o".to_owned())));
    match body.get("messages") {
        Some(Json::List(rows)) => assert_eq!(rows.len(), 1),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => panic!("the provider body carries the conversation"),
    }
}

#[test]
fn a_chat_answer_is_shaped_as_chat() {
    let data = object(
        r#"{"response":{"id":"r1","object":"chat.completion","created":5,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}]}}"#,
    );
    assert!(matches!(response_shape(&data), Ok(Response::Chat(_))));
}

#[test]
fn a_text_completion_answer_is_shaped_as_one() {
    let data = object(
        r#"{"response":{"id":"cmpl-1","object":"text_completion","created":5,"model":"davinci","choices":[{"index":0,"finish_reason":"stop","text":"hi"}]}}"#,
    );
    assert!(matches!(
        response_shape(&data),
        Ok(Response::TextCompletion(_))
    ));
}

#[test]
fn an_answer_that_is_not_an_object_stays_opaque() {
    let data = object(r#"{"response":"plain"}"#);
    assert!(matches!(response_shape(&data), Ok(Response::Other(_))));
}
