#![cfg(test)]

use crate::routes::chat::chat_routes;
use crate::server::chat_setup::chat_executor;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request};
use betterlite_config::loader::ConfigLoader;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout};
use tower::ServiceExt as _;

const ANSWER: &str = r#"{"id":"c1","object":"chat.completion","created":7,"model":"upstream-model","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}]}"#;
const LIMITS: &str = "x-ratelimit-remaining-requests: 9\r\nx-ratelimit-reset-requests: 1s\r\n";
const NO_DELAY: Duration = Duration::ZERO;

struct Exchange {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
    sent: String,
}

#[derive(Deserialize)]
struct Completion {
    model: String,
}

#[derive(Deserialize)]
struct ErrorReply {
    error: ErrorFields,
}

#[derive(Deserialize)]
struct ErrorFields {
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: String,
    provider_specific_fields: Option<RouterFields>,
}

#[derive(Deserialize)]
struct RouterFields {
    error: String,
}

async fn provider(listener: TcpListener, reply: (u16, &str), delay: Duration) -> String {
    let Ok(Ok((mut stream, _))) = timeout(Duration::from_secs(1), listener.accept()).await else {
        return String::new();
    };
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(stream.read_u8().await.unwrap());
    }
    let mut body = vec![0_u8; 4096];
    let read = stream.read(&mut body).await.unwrap();
    sleep(delay).await;
    let (status, answer) = reply;
    let head = format!(
        "HTTP/1.1 {status} X\r\n{LIMITS}content-length: {}\r\n\r\n",
        answer.len()
    );
    let _written = stream.write_all(format!("{head}{answer}").as_bytes()).await;
    body.truncate(read);
    received.extend(body);
    String::from_utf8(received).unwrap()
}

async fn exchange(
    path: &str,
    request: &str,
    call_id: &str,
    reply: (u16, &str),
    delay: Duration,
) -> Exchange {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let source = format!(
        "model_list:\n  - {{model_name: g, litellm_params: {{model: openai/up, api_base: '{base}', timeout: 0.4}}, model_info: {{id: dep-1}}}}\n"
    );
    let environment = BTreeMap::from([
        ("OPENAI_API_KEY".to_owned(), "env-key".to_owned()),
        ("OPENAI_ORGANIZATION".to_owned(), "org-9".to_owned()),
    ]);
    let loader = ConfigLoader {
        lookup: environment.clone(),
    };
    let config = loader.load(&source).unwrap();
    let executor = chat_executor(&config, &environment, StdRng::seed_from_u64(3)).unwrap();
    let mut builder = Request::post(path).header("content-type", "application/json");
    if !call_id.is_empty() {
        builder = builder.header("x-litellm-call-id", call_id);
    }
    let call = chat_routes(Arc::new(executor))
        .oneshot(builder.body(Body::from(request.to_owned())).unwrap());
    let (sent, answered) = tokio::join!(provider(listener, reply, delay), call);
    let response = answered.unwrap();
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    Exchange {
        status,
        headers,
        body,
        sent,
    }
}

fn header<'map>(headers: &'map HeaderMap, name: &str) -> Option<&'map str> {
    headers.get(name).map(|value| value.to_str().unwrap())
}

#[rstest]
#[case::chat("/chat/completions", r#"{"model":"g","messages":[]}"#, "g")]
#[case::v1("/v1/chat/completions", r#"{"model":"g","messages":[]}"#, "g")]
#[case::engines_path_wins(
    "/engines/g/chat/completions",
    r#"{"model":"not-configured","messages":[]}"#,
    "not-configured"
)]
#[case::deployments_without_body_model(
    "/openai/deployments/g/chat/completions",
    r#"{"messages":[]}"#,
    "upstream-model"
)]
#[tokio::test]
async fn serves_every_chat_route_with_litellm_headers(
    #[case] path: &str,
    #[case] request: &str,
    #[case] model: &str,
) {
    let answered = exchange(path, request, "call-1", (200, ANSWER), NO_DELAY).await;
    assert_eq!(answered.status, 200);
    assert_eq!(
        serde_json::from_slice::<Completion>(&answered.body)
            .unwrap()
            .model,
        model
    );
    assert!(
        answered.sent.contains("authorization: Bearer env-key")
            && answered.sent.contains("openai-organization: org-9")
    );
    let expected = [
        ("x-litellm-call-id", "call-1"),
        ("x-litellm-model-id", "dep-1"),
        ("x-litellm-model-name", "openai/up"),
        ("x-litellm-model-group", "g"),
        ("x-litellm-callback-duration-ms", "0.0"),
        ("x-litellm-attempted-retries", "0"),
        ("x-litellm-attempted-fallbacks", "0"),
        ("x-litellm-version", env!("CARGO_PKG_VERSION")),
        ("llm_provider-x-ratelimit-reset-requests", "1s"),
        ("x-ratelimit-reset-requests", "1s"),
        ("x-ratelimit-remaining-requests", "9"),
    ];
    for (name, value) in expected {
        assert_eq!(header(&answered.headers, name), Some(value), "{name}");
    }
    assert!(
        header(&answered.headers, "x-litellm-model-api-base")
            .unwrap()
            .ends_with("/v1")
    );
    assert!(header(&answered.headers, "x-litellm-timeout").is_none());
    for name in [
        "x-litellm-response-duration-ms",
        "x-litellm-overhead-duration-ms",
    ] {
        let (whole, fraction) = header(&answered.headers, name)
            .unwrap()
            .split_once('.')
            .unwrap();
        assert!(
            whole.parse::<u64>().is_ok() && fraction.len() == 3 && fraction.parse::<u16>().is_ok(),
            "{name}"
        );
    }
}

#[rstest]
#[case::missing("", 36)]
#[case::too_long(&"c".repeat(257), 36)]
#[case::longest_kept(&"c".repeat(256), 256)]
#[tokio::test]
async fn echoes_a_valid_call_id_and_generates_one_otherwise(
    #[case] call_id: &str,
    #[case] length: usize,
) {
    let answered = exchange(
        "/chat/completions",
        r#"{"model":"g","messages":[]}"#,
        call_id,
        (200, ANSWER),
        NO_DELAY,
    )
    .await;
    assert_eq!(
        header(&answered.headers, "x-litellm-call-id")
            .unwrap()
            .len(),
        length
    );
}

#[rstest]
#[case::unknown_model(r#"{"model":"missing","messages":[]}"#, (200, ANSWER, NO_DELAY), (400, "invalid_request_error", None, None))]
#[case::bad_body(r#"{"model":"g"}"#, (200, ANSWER, NO_DELAY), (400, "invalid_request_error", None, None))]
#[case::rate_limited(r#"{"model":"g","messages":[]}"#, (429, r#"{"error":{"type":"requests","param":"p"}}"#, NO_DELAY), (429, "throttling_error", Some("p"), Some("0.4")))]
#[case::slower_than_the_timeout(r#"{"model":"g","messages":[]}"#, (200, ANSWER, Duration::from_secs(2)), (408, "invalid_request_error", None, Some("0.4")))]
#[tokio::test]
async fn answers_failures_in_litellm_shape(
    #[case] request: &str,
    #[case] upstream: (u16, &str, Duration),
    #[case] expected: (u16, &str, Option<&str>, Option<&str>),
) {
    let (reply_status, reply, delay) = upstream;
    let (status, error_type, param, timeout_header) = expected;
    let answered = exchange(
        "/v1/chat/completions",
        request,
        "call-2",
        (reply_status, reply),
        delay,
    )
    .await;
    assert_eq!(answered.status, status);
    let error = serde_json::from_slice::<ErrorReply>(&answered.body)
        .unwrap()
        .error;
    assert_eq!(error.error_type, error_type);
    assert_eq!(error.code, status.to_string());
    assert_eq!(error.param.as_deref(), param);
    let repeated = error.provider_specific_fields.map(|fields| fields.error);
    assert_eq!(repeated.is_some(), request.contains("missing"));
    assert_eq!(
        header(&answered.headers, "x-litellm-call-id"),
        Some("call-2")
    );
    assert_eq!(
        header(&answered.headers, "x-litellm-timeout"),
        timeout_header
    );
    let forwarded = status == 429;
    assert_eq!(
        header(&answered.headers, "x-ratelimit-remaining-requests").is_some(),
        forwarded
    );
    assert_eq!(
        header(&answered.headers, "llm_provider-x-ratelimit-reset-requests").is_some(),
        forwarded
    );
    assert!(header(&answered.headers, "x-ratelimit-reset-requests").is_none());
}
