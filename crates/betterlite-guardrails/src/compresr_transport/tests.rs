#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::error::CompresrTransportError;
use super::{
    CompresrTransport, CompressOutcome, CompressTarget, CompressionResult, TransportSettings,
};
use crate::generic_guardrail_contracts::UnreachableFallback;

type Shared = Arc<Mutex<Vec<Recorded>>>;

#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Option<GuardrailValue>,
}

async fn record(
    State((recorded, status, reply)): State<(Shared, StatusCode, String)>,
    request: Request,
) -> (StatusCode, String) {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, 1024 * 1024).await.expect("body");
    let headers = parts
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    let parsed = String::from_utf8(bytes.to_vec())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    recorded.lock().expect("recorded").push(Recorded {
        method: parts.method.as_str().to_owned(),
        path: parts.uri.path().to_owned(),
        headers,
        body: parsed,
    });
    (status, reply)
}

async fn serve(status: u16, reply: &str) -> (String, Shared, JoinSet<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let recorded: Shared = Arc::new(Mutex::new(Vec::new()));
    let state = (
        Arc::clone(&recorded),
        StatusCode::from_u16(status).expect("status"),
        reply.to_owned(),
    );
    let router = Router::new().fallback(record).with_state(state);
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, recorded, server)
}

fn settings(api_base: &str) -> TransportSettings {
    TransportSettings {
        api_base: api_base.to_owned(),
        model: "latte_v2".to_owned(),
        target_compression_ratio: 0.5,
        coarse: true,
        dynamic: true,
        dynamic_min_ratio: None,
        dynamic_max_ratio: None,
        compression_params: BTreeMap::new(),
        timeout: Duration::from_secs(5),
        fallback: UnreachableFallback::FailClosed,
    }
}

fn transport(api_base: &str) -> CompresrTransport {
    CompresrTransport::new(
        settings(api_base),
        SecretString::from("test-key"),
        reqwest::Client::default(),
    )
    .expect("transport")
}

fn target(context: &str, query: &str) -> CompressTarget {
    CompressTarget {
        context: context.to_owned(),
        query: query.to_owned(),
    }
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn field(body: &GuardrailValue, name: &str) -> GuardrailValue {
    let GuardrailValue::Object(fields) = body else {
        panic!("the request body is not an object")
    };
    fields.get(name).expect("request field").clone()
}

fn has_field(body: &GuardrailValue, name: &str) -> bool {
    let GuardrailValue::Object(fields) = body else {
        panic!("the request body is not an object")
    };
    fields.contains_key(name)
}

fn calls(recorded: &Shared) -> Vec<Recorded> {
    recorded.lock().expect("recorded").clone()
}

fn result(compressed: &str, before: i64, after: i64) -> CompressionResult {
    CompressionResult {
        compressed_context: Some(compressed.to_owned()),
        original_tokens: before,
        compressed_tokens: after,
    }
}

#[tokio::test]
async fn a_single_context_posts_to_the_single_endpoint() {
    let (base, recorded, _server) = serve(
        200,
        r#"{"data":{"compressed_context":"short","original_tokens":9,"compressed_tokens":3}}"#,
    )
    .await;
    let outcome = transport(&base)
        .compress(&[target("the long context", "the query")])
        .await
        .expect("outcome");
    assert_eq!(
        outcome,
        CompressOutcome::Results(vec![result("short", 9, 3)])
    );
    let calls = calls(&recorded);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "POST");
    assert_eq!(calls[0].path, "/api/compress/question-specific/");
    let body = calls[0].body.as_ref().expect("request body");
    assert_eq!(field(body, "context"), text("the long context"));
    assert_eq!(field(body, "query"), text("the query"));
    assert_eq!(field(body, "compression_model_name"), text("latte_v2"));
    assert_eq!(field(body, "source"), text("integration:litellm"));
    assert_eq!(field(body, "coarse"), GuardrailValue::Boolean(true));
    assert_eq!(
        field(body, "target_compression_ratio"),
        GuardrailValue::Decimal(0.5)
    );
    assert!(!has_field(body, "inputs"));
    assert!(!has_field(body, "dynamic_min_ratio"));
    assert!(!has_field(body, "dynamic_max_ratio"));
}

#[tokio::test]
async fn several_contexts_post_one_batch_input_per_target() {
    let (base, recorded, _server) = serve(
        200,
        r#"{"data":{"results":[
            {"compressed_context":"one","original_tokens":5,"compressed_tokens":1},
            {"compressed_context":"two","original_tokens":6,"compressed_tokens":2}
        ]}}"#,
    )
    .await;
    let outcome = transport(&base)
        .compress(&[target("first", "q1"), target("second", "q2")])
        .await
        .expect("outcome");
    assert_eq!(
        outcome,
        CompressOutcome::Results(vec![result("one", 5, 1), result("two", 6, 2)])
    );
    let calls = calls(&recorded);
    assert_eq!(calls[0].path, "/api/compress/question-specific/batch");
    let body = calls[0].body.as_ref().expect("request body");
    let GuardrailValue::Array(inputs) = field(body, "inputs") else {
        panic!("inputs is not a list")
    };
    assert_eq!(inputs.len(), 2);
    assert_eq!(field(&inputs[0], "context"), text("first"));
    assert_eq!(field(&inputs[0], "query"), text("q1"));
    assert_eq!(field(&inputs[1], "context"), text("second"));
    assert_eq!(field(&inputs[1], "query"), text("q2"));
    assert!(!has_field(body, "context"));
    assert!(!has_field(body, "query"));
}

#[tokio::test]
async fn the_api_key_and_json_content_type_are_sent_as_headers() {
    let (base, recorded, _server) = serve(200, r#"{"data":{}}"#).await;
    transport(&base)
        .compress(&[target("context", "query")])
        .await
        .expect("outcome");
    let calls = calls(&recorded);
    assert_eq!(
        calls[0].headers.get("content-type"),
        Some(&"application/json".to_owned())
    );
    assert_eq!(
        calls[0].headers.get("x-api-key"),
        Some(&"test-key".to_owned())
    );
}

#[tokio::test]
async fn the_optional_ratios_are_sent_only_when_set() {
    let (base, recorded, _server) = serve(200, r#"{"data":{}}"#).await;
    let mut built = settings(&base);
    built.dynamic_min_ratio = Some(0.2);
    built.dynamic_max_ratio = Some(0.8);
    let transport = CompresrTransport::new(
        built,
        SecretString::from("test-key"),
        reqwest::Client::default(),
    )
    .expect("transport");
    transport
        .compress(&[target("context", "query")])
        .await
        .expect("outcome");
    let calls = calls(&recorded);
    let body = calls[0].body.as_ref().expect("request body");
    assert_eq!(
        field(body, "dynamic_min_ratio"),
        GuardrailValue::Decimal(0.2)
    );
    assert_eq!(
        field(body, "dynamic_max_ratio"),
        GuardrailValue::Decimal(0.8)
    );
}

#[tokio::test]
async fn passthrough_params_are_sent_but_never_replace_the_request_content() {
    let (base, recorded, _server) = serve(200, r#"{"data":{}}"#).await;
    let mut built = settings(&base);
    built.compression_params = BTreeMap::from([
        ("context".to_owned(), text("hijacked context")),
        ("query".to_owned(), text("hijacked query")),
        ("inputs".to_owned(), text("hijacked inputs")),
        ("temperature".to_owned(), GuardrailValue::Decimal(0.2)),
    ]);
    let transport = CompresrTransport::new(
        built,
        SecretString::from("test-key"),
        reqwest::Client::default(),
    )
    .expect("transport");
    transport
        .compress(&[target("the context", "the query")])
        .await
        .expect("outcome");
    let calls = calls(&recorded);
    let body = calls[0].body.as_ref().expect("request body");
    assert_eq!(field(body, "context"), text("the context"));
    assert_eq!(field(body, "query"), text("the query"));
    assert_eq!(field(body, "temperature"), GuardrailValue::Decimal(0.2));
    assert!(!has_field(body, "inputs"));
}

#[tokio::test]
async fn a_passthrough_key_the_request_names_is_replaced_by_the_named_value() {
    let (base, recorded, _server) = serve(200, r#"{"data":{}}"#).await;
    let mut built = settings(&base);
    built.dynamic_min_ratio = Some(0.25);
    built.compression_params = BTreeMap::from([
        ("coarse".to_owned(), GuardrailValue::Boolean(false)),
        ("source".to_owned(), text("hijacked source")),
        ("compression_model_name".to_owned(), text("other_model")),
        (
            "target_compression_ratio".to_owned(),
            GuardrailValue::Decimal(0.9),
        ),
        ("dynamic".to_owned(), GuardrailValue::Boolean(false)),
        (
            "dynamic_min_ratio".to_owned(),
            GuardrailValue::Decimal(0.75),
        ),
        ("dynamic_max_ratio".to_owned(), GuardrailValue::Decimal(0.8)),
    ]);
    let transport = CompresrTransport::new(
        built,
        SecretString::from("test-key"),
        reqwest::Client::default(),
    )
    .expect("transport");
    transport
        .compress(&[target("the context", "the query")])
        .await
        .expect("outcome");
    let calls = calls(&recorded);
    let body = calls[0].body.as_ref().expect("request body");
    assert_eq!(field(body, "coarse"), GuardrailValue::Boolean(true));
    assert_eq!(field(body, "source"), text("integration:litellm"));
    assert_eq!(field(body, "compression_model_name"), text("latte_v2"));
    assert_eq!(
        field(body, "target_compression_ratio"),
        GuardrailValue::Decimal(0.5)
    );
    assert_eq!(field(body, "dynamic"), GuardrailValue::Boolean(true));
    assert_eq!(
        field(body, "dynamic_min_ratio"),
        GuardrailValue::Decimal(0.25)
    );
    assert_eq!(
        field(body, "dynamic_max_ratio"),
        GuardrailValue::Decimal(0.8)
    );
}

#[tokio::test]
async fn a_passthrough_ratio_bound_survives_when_the_other_bound_is_the_one_set() {
    let (base, recorded, _server) = serve(200, r#"{"data":{}}"#).await;
    let mut built = settings(&base);
    built.dynamic_max_ratio = Some(0.9);
    built.compression_params = BTreeMap::from([
        (
            "dynamic_min_ratio".to_owned(),
            GuardrailValue::Decimal(0.75),
        ),
        ("dynamic_max_ratio".to_owned(), GuardrailValue::Decimal(0.8)),
    ]);
    let transport = CompresrTransport::new(
        built,
        SecretString::from("test-key"),
        reqwest::Client::default(),
    )
    .expect("transport");
    transport
        .compress(&[target("the context", "the query")])
        .await
        .expect("outcome");
    let calls = calls(&recorded);
    let body = calls[0].body.as_ref().expect("request body");
    assert_eq!(
        field(body, "dynamic_min_ratio"),
        GuardrailValue::Decimal(0.75)
    );
    assert_eq!(
        field(body, "dynamic_max_ratio"),
        GuardrailValue::Decimal(0.9)
    );
}

const ONE_MIB: usize = 1024 * 1024;

fn reply_of_length(total: usize) -> String {
    let prefix = r#"{"data":{"compressed_context":""#;
    let suffix = r#""}}"#;
    let padding = "a".repeat(total - prefix.len() - suffix.len());
    format!("{prefix}{padding}{suffix}")
}

#[tokio::test]
async fn a_reply_of_exactly_the_size_limit_is_read() {
    let (base, _recorded, _server) = serve(200, &reply_of_length(ONE_MIB)).await;
    let outcome = transport(&base)
        .compress(&[target("context", "query")])
        .await
        .expect("outcome");
    let CompressOutcome::Results(results) = outcome else {
        panic!("the reply at the limit was not read");
    };
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn a_reply_over_the_size_limit_goes_through_the_fail_policy() {
    let (base, _recorded, _server) = serve(200, &reply_of_length(ONE_MIB + 1)).await;
    let mut built = settings(&base);
    built.fallback = UnreachableFallback::FailOpen;
    let transport = CompresrTransport::new(
        built,
        SecretString::from("test-key"),
        reqwest::Client::default(),
    )
    .expect("transport");
    let outcome = transport
        .compress(&[target("context", "query")])
        .await
        .expect("outcome");
    assert_eq!(outcome, CompressOutcome::Uncompressed);
}

#[tokio::test]
async fn a_trailing_slash_on_the_api_base_does_not_double_the_path() {
    let (base, recorded, _server) = serve(200, r#"{"data":{}}"#).await;
    transport(&format!("{base}/"))
        .compress(&[target("context", "query")])
        .await
        .expect("outcome");
    let calls = calls(&recorded);
    assert_eq!(calls[0].path, "/api/compress/question-specific/");
}

#[tokio::test]
async fn fail_closed_raises_a_502_for_an_upstream_error() {
    let (base, recorded, _server) = serve(500, r#"{"error":"upstream secret detail"}"#).await;
    let error = transport(&base)
        .compress(&[target("context", "query")])
        .await
        .expect_err("failure");
    assert_eq!(error.status_code(), Some(502));
    assert_eq!(
        error,
        CompresrTransportError::CompressionFailed {
            message: "Compresr compression service returned an error"
        }
    );
    assert_eq!(calls(&recorded).len(), 1);
}

#[tokio::test]
async fn fail_open_forwards_the_request_uncompressed_after_an_upstream_error() {
    let (base, _recorded, _server) = serve(500, r#"{"error":"upstream"}"#).await;
    let mut built = settings(&base);
    built.fallback = UnreachableFallback::FailOpen;
    let transport = CompresrTransport::new(
        built,
        SecretString::from("test-key"),
        reqwest::Client::default(),
    )
    .expect("transport");
    let outcome = transport
        .compress(&[target("context", "query")])
        .await
        .expect("outcome");
    assert_eq!(outcome, CompressOutcome::Uncompressed);
}

#[tokio::test]
async fn fail_open_treats_an_unreadable_body_as_a_failed_compression() {
    let (base, _recorded, _server) = serve(200, "{not json").await;
    let mut built = settings(&base);
    built.fallback = UnreachableFallback::FailOpen;
    let transport = CompresrTransport::new(
        built,
        SecretString::from("test-key"),
        reqwest::Client::default(),
    )
    .expect("transport");
    let outcome = transport
        .compress(&[target("context", "query"), target("other", "query")])
        .await
        .expect("outcome");
    assert_eq!(outcome, CompressOutcome::Uncompressed);
}

#[tokio::test]
async fn a_batch_with_the_wrong_number_of_results_raises_when_fail_closed() {
    let (base, _recorded, _server) = serve(
        200,
        r#"{"data":{"results":[{"compressed_context":"only one"}]}}"#,
    )
    .await;
    let error = transport(&base)
        .compress(&[target("first", "q1"), target("second", "q2")])
        .await
        .expect_err("failure");
    assert_eq!(error.status_code(), Some(502));
}

#[tokio::test]
async fn a_batch_with_the_wrong_number_of_results_is_forwarded_when_fail_open() {
    let (base, _recorded, _server) = serve(
        200,
        r#"{"data":{"results":[{"compressed_context":"only one"}]}}"#,
    )
    .await;
    let mut built = settings(&base);
    built.fallback = UnreachableFallback::FailOpen;
    let transport = CompresrTransport::new(
        built,
        SecretString::from("test-key"),
        reqwest::Client::default(),
    )
    .expect("transport");
    let outcome = transport
        .compress(&[target("first", "q1"), target("second", "q2")])
        .await
        .expect("outcome");
    assert_eq!(outcome, CompressOutcome::Uncompressed);
}

#[tokio::test]
async fn malformed_token_stats_on_a_successful_response_do_not_raise() {
    let (base, _recorded, _server) = serve(
        200,
        r#"{"data":{"compressed_context":"short","original_tokens":"lots","compressed_tokens":null}}"#,
    )
    .await;
    let outcome = transport(&base)
        .compress(&[target("context", "query")])
        .await
        .expect("outcome");
    assert_eq!(
        outcome,
        CompressOutcome::Results(vec![result("short", 0, 0)])
    );
}

#[test]
fn a_cloud_metadata_api_base_is_refused_before_any_request() {
    assert_eq!(
        CompresrTransport::new(
            settings("http://169.254.169.254"),
            SecretString::from("test-key"),
            reqwest::Client::default(),
        )
        .expect_err("blocked"),
        CompresrTransportError::BlockedHost {
            host: "169.254.169.254".to_owned()
        }
    );
}
