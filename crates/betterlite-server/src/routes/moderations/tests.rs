#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_util::task::TaskTracker;
use tower::ServiceExt as _;

use crate::routes::tests::state_for;
use crate::server::application;
use crate::server::chat_setup::chat_executor;
use crate::state::ServerState;

use super::routes;

const CATEGORIES: [&str; 13] = [
    "harassment",
    "harassment/threatening",
    "hate",
    "hate/threatening",
    "illicit",
    "illicit/violent",
    "self-harm",
    "self-harm/instructions",
    "self-harm/intent",
    "sexual",
    "sexual/minors",
    "violence",
    "violence/graphic",
];

const PROVIDER_400: &str = r#"{"error":{"message":"Invalid input","type":"invalid_request_error","param":"input","code":"invalid_input"}}"#;
const PROVIDER_401: &str = r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error","code":"invalid_api_key"}}"#;
const PROVIDER_429: &str = r#"{"error":{"message":"Rate limit reached for requests","type":"tokens","code":"rate_limit_exceeded"}}"#;
const PROVIDER_500: &str =
    r#"{"error":{"message":"The server had an error processing your request."}}"#;

fn config(source: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("config")
}

fn repeated(value: &Value) -> Map<String, Value> {
    CATEGORIES
        .iter()
        .map(|name| ((*name).to_owned(), value.clone()))
        .collect()
}

// the provider answers every category, so the route has a full response to rewrite
fn answer(score: &str) -> String {
    let number: Value = serde_json::from_str(score).expect("score lexeme");
    serde_json::to_string(&json!({
        "id": "modr-1",
        "model": "omni-moderation-latest",
        "results": [{
            "categories": repeated(&json!(false)),
            "category_applied_input_types": repeated(&json!(["text"])),
            "category_scores": repeated(&number),
            "flagged": false,
        }],
    }))
    .expect("answer body")
}

fn state_with(guard_base: &str, other_base: &str, extra: &str) -> Arc<ServerState> {
    let source = format!(
        "{extra}model_list:\n  - model_name: guard\n    litellm_params: {{model: openai/omni-moderation-latest, api_base: '{guard_base}', api_key: oracle-key}}\n  - model_name: other\n    litellm_params: {{model: openai/other-moderation, api_base: '{other_base}', api_key: other-key}}\n"
    );
    state_for(&config(&source))
}

fn request(path: &str, body: &str) -> Request<Body> {
    Request::post(path)
        .header("content-type", "application/json")
        .header("authorization", "Bearer oracle-master")
        .header("x-litellm-call-id", "caller-id")
        .body(Body::from(body.to_owned()))
        .expect("request")
}

fn header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn content_length(head: &[u8]) -> usize {
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    text.lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

async fn record_request(listener: TcpListener, reply: (u16, String)) -> String {
    let Ok(Ok((mut stream, _))) = timeout(Duration::from_millis(800), listener.accept()).await
    else {
        return String::new();
    };
    let received = read_request(&mut stream).await;
    let (status, body) = reply;
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\ncontent-type: application/json\r\nx-request-id: provider-call\r\n\r\n",
        body.len()
    );
    let _written = stream.write_all(format!("{head}{body}").as_bytes()).await;
    String::from_utf8_lossy(&received).into_owned()
}

async fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut received = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let Ok(read) = stream.read(&mut buffer).await else {
            break;
        };
        if read == 0 {
            break;
        }
        received.extend_from_slice(&buffer[..read]);
        if let Some(end) = header_end(&received)
            && received.len() >= end + content_length(&received[..end])
        {
            break;
        }
    }
    received
}

// the provider accepts and never answers, so only the deployment timeout ends the call
async fn hang(listener: TcpListener) {
    let Ok(Ok((mut stream, _))) = timeout(Duration::from_secs(2), listener.accept()).await else {
        return;
    };
    let mut buffer = [0_u8; 1024];
    let _read = stream.read(&mut buffer).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
}

#[derive(Debug)]
struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
    raw: String,
    sent: String,
}

async fn exchange<Provider>(router: Router, request: Request<Body>, provider: Provider) -> Reply
where
    Provider: Future<Output = String>,
{
    let (response, sent) = tokio::join!(router.oneshot(request), provider);
    let response = response.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.expect("body");
    let raw = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Reply {
        status,
        headers,
        body,
        raw,
        sent,
    }
}

async fn call(
    extra: &str,
    base_suffix: &str,
    path: &str,
    body: &str,
    reply: (u16, String),
) -> Reply {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let guard = format!("http://{address}{base_suffix}");
    let other = format!("http://{address}/other/v1");
    let state = state_with(&guard, &other, extra);
    exchange(
        routes(&state),
        request(path, body),
        record_request(listener, reply),
    )
    .await
}

fn error_of(reply: &Reply) -> &Value {
    &reply.body["error"]
}

fn call_id(reply: &Reply) -> Option<&str> {
    reply
        .headers
        .get("x-litellm-call-id")
        .and_then(|value| value.to_str().ok())
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    StringInput,
    ArrayInput,
    MultimodalInput,
    ConfiguredDefault,
    ConfiguredDefaultOverridesBody,
    ExplicitOtherModel,
    UnversionedPath,
    Provider400,
    Provider401,
    Provider429,
    Provider500,
    DecimalScore,
    SmallestScore,
    MalformedBody,
    MissingInput,
    ModelRequired,
    UnknownModel,
    RequestSuppliedDestination,
    CallIdInErrorBody,
    CallIdLeftOutOfErrorBody,
    AuthRequired,
    BodyLimit,
    DeploymentTimeout,
}

#[rstest]
#[case::string_input(Scenario::StringInput)]
#[case::array_input(Scenario::ArrayInput)]
#[case::multimodal_input(Scenario::MultimodalInput)]
#[case::configured_default(Scenario::ConfiguredDefault)]
#[case::configured_default_overrides_body(Scenario::ConfiguredDefaultOverridesBody)]
#[case::explicit_other_model(Scenario::ExplicitOtherModel)]
#[case::unversioned_path(Scenario::UnversionedPath)]
#[case::provider_400(Scenario::Provider400)]
#[case::provider_401(Scenario::Provider401)]
#[case::provider_429(Scenario::Provider429)]
#[case::provider_500(Scenario::Provider500)]
#[case::decimal_score(Scenario::DecimalScore)]
#[case::smallest_score(Scenario::SmallestScore)]
#[case::malformed_body(Scenario::MalformedBody)]
#[case::missing_input(Scenario::MissingInput)]
#[case::model_required(Scenario::ModelRequired)]
#[case::unknown_model(Scenario::UnknownModel)]
#[case::request_supplied_destination(Scenario::RequestSuppliedDestination)]
#[case::call_id_in_error_body(Scenario::CallIdInErrorBody)]
#[case::call_id_left_out_of_error_body(Scenario::CallIdLeftOutOfErrorBody)]
#[case::auth_required(Scenario::AuthRequired)]
#[case::body_limit(Scenario::BodyLimit)]
#[case::deployment_timeout(Scenario::DeploymentTimeout)]
#[tokio::test]
async fn moderations_moderation_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::StringInput => string_input().await,
        Scenario::ArrayInput => array_input().await,
        Scenario::MultimodalInput => multimodal_input().await,
        Scenario::ConfiguredDefault => configured_default().await,
        Scenario::ConfiguredDefaultOverridesBody => configured_default_overrides_body().await,
        Scenario::ExplicitOtherModel => explicit_other_model().await,
        Scenario::UnversionedPath => unversioned_path().await,
        Scenario::Provider400 => provider_error(400, PROVIDER_400).await,
        Scenario::Provider401 => provider_error(401, PROVIDER_401).await,
        Scenario::Provider429 => provider_error(429, PROVIDER_429).await,
        Scenario::Provider500 => provider_error(500, PROVIDER_500).await,
        Scenario::DecimalScore => decimal_score().await,
        Scenario::SmallestScore => smallest_score().await,
        Scenario::MalformedBody => malformed_body().await,
        Scenario::MissingInput => missing_input().await,
        Scenario::ModelRequired => model_required().await,
        Scenario::UnknownModel => unknown_model().await,
        Scenario::RequestSuppliedDestination => request_supplied_destination().await,
        Scenario::CallIdInErrorBody => call_id_in_error_body().await,
        Scenario::CallIdLeftOutOfErrorBody => call_id_left_out_of_error_body().await,
        Scenario::AuthRequired => auth_required().await,
        Scenario::BodyLimit => body_limit().await,
        Scenario::DeploymentTimeout => deployment_timeout().await,
    }
}

fn assert_success(reply: &Reply) {
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.raw);
    assert_eq!(
        reply
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(call_id(reply), Some("caller-id"));
    // the moderation handler leaves get_custom_headers no provider headers
    assert!(
        !reply.headers.contains_key("llm_provider-content-type"),
        "{:?}",
        reply.headers
    );
    assert_eq!(reply.body["id"], json!("modr-1"));
    assert_eq!(reply.body["model"], json!("omni-moderation-latest"));
    assert_eq!(reply.body["results"][0]["flagged"], json!(false));
}

fn assert_sent_model(reply: &Reply, model: &str) {
    let expected = format!(r#""model":"{model}"#);
    assert!(reply.sent.contains(&expected), "{}", reply.sent);
}

async fn string_input() {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"model":"guard","input":"hello"}"#,
        (200, answer("0.0001")),
    )
    .await;
    assert_success(&reply);
    assert!(
        reply
            .sent
            .contains(r#"{"input":"hello","model":"omni-moderation-latest"}"#),
        "{}",
        reply.sent
    );
    let categories = &reply.body["results"][0]["categories"];
    assert_eq!(categories["harassment"], json!(false));
    assert_eq!(categories["harassment_threatening"], json!(false));
    // LiteLLM dumps the SDK model's field names, so the provider's slash key never comes back
    assert_eq!(categories["harassment/threatening"], Value::Null);
    assert_eq!(categories["self_harm"], json!(false));
    assert_eq!(categories["self-harm"], Value::Null);
    assert_eq!(
        reply.body["results"][0]["category_scores"]["violence_graphic"],
        json!(0.0001)
    );
}

async fn array_input() {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"model":"guard","input":["a","b"]}"#,
        (200, answer("0.5")),
    )
    .await;
    assert_success(&reply);
    assert!(
        reply
            .sent
            .contains(r#"{"input":["a","b"],"model":"omni-moderation-latest"}"#),
        "{}",
        reply.sent
    );
}

async fn multimodal_input() {
    let reply = call(
        "",
        "/v1",
        "/moderations",
        r#"{"model":"guard","input":[{"type":"image_url","image_url":{"url":"https://example.test/image.png"}},{"type":"text","text":"hello"}]}"#,
        (200, answer("0.5")),
    )
    .await;
    assert_success(&reply);
    assert!(
        reply.sent.contains(
            r#"{"image_url":{"url":"https://example.test/image.png"},"type":"image_url"}"#
        ),
        "{}",
        reply.sent
    );
    assert!(
        reply.sent.contains(r#"{"text":"hello","type":"text"}"#),
        "{}",
        reply.sent
    );
}

async fn configured_default() {
    let reply = call(
        "general_settings: {moderation_model: guard}\n",
        "/v1",
        "/v1/moderations",
        r#"{"input":"hello"}"#,
        (200, answer("0.5")),
    )
    .await;
    assert_success(&reply);
    assert_sent_model(&reply, "omni-moderation-latest");
    assert!(
        reply.sent.starts_with("POST /v1/moderations "),
        "{}",
        reply.sent
    );
}

async fn configured_default_overrides_body() {
    let reply = call(
        "general_settings: {moderation_model: guard}\n",
        "/v1",
        "/v1/moderations",
        r#"{"model":"other","input":"hello"}"#,
        (200, answer("0.5")),
    )
    .await;
    assert_success(&reply);
    assert_sent_model(&reply, "omni-moderation-latest");
    assert!(
        reply.sent.starts_with("POST /v1/moderations "),
        "the configured default picks the guard deployment: {}",
        reply.sent
    );
}

async fn explicit_other_model() {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"model":"other","input":"hello"}"#,
        (200, answer("0.5")),
    )
    .await;
    assert_success(&reply);
    assert_sent_model(&reply, "other-moderation");
    assert!(
        reply.sent.starts_with("POST /other/v1/moderations "),
        "{}",
        reply.sent
    );
}

async fn unversioned_path() {
    let reply = call(
        "",
        "",
        "/moderations",
        r#"{"model":"guard","input":"hello"}"#,
        (200, answer("0.5")),
    )
    .await;
    assert_success(&reply);
    assert!(
        reply.sent.starts_with("POST /moderations "),
        "{}",
        reply.sent
    );
}

async fn provider_error(status: u16, body: &str) {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"model":"guard","input":"hello"}"#,
        (status, body.to_owned()),
    )
    .await;
    assert_eq!(reply.status.as_u16(), status, "{}", reply.raw);
    let code = status.to_string();
    assert_eq!(error_of(&reply)["code"], json!(code));
    assert_eq!(call_id(&reply), Some("caller-id"));
    assert!(
        !reply.headers.contains_key("llm_provider-content-type"),
        "the moderation except answers litellm_call_id_headers alone: {:?}",
        reply.headers
    );
    assert!(!reply.headers.contains_key("x-litellm-timeout"));
    match status {
        400 => {
            assert_eq!(error_of(&reply)["type"], json!("invalid_request_error"));
            assert_eq!(error_of(&reply)["param"], json!("input"));
        }
        401 => {
            assert_eq!(error_of(&reply)["type"], Value::Null);
            assert_eq!(error_of(&reply)["param"], Value::Null);
        }
        429 => {
            assert_eq!(error_of(&reply)["type"], json!("throttling_error"));
            assert_eq!(error_of(&reply)["param"], Value::Null);
        }
        500 => {
            assert_eq!(error_of(&reply)["type"], json!("internal_server_error"));
            assert_eq!(error_of(&reply)["param"], Value::Null);
        }
        other => panic!("unexpected provider status {other}"),
    }
}

async fn decimal_score() {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"model":"guard","input":"hello"}"#,
        (200, answer("1e-29")),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY, "{}", reply.raw);
    assert_eq!(
        reply.raw,
        r#"{"error":{"message":"the category_scores value for harassment is not exactly representable as a Decimal: Scale exceeds the maximum precision allowed: 29 > 28","type":"bad_gateway","param":"category_scores","code":"decimal_out_of_range"}}"#
    );
}

async fn smallest_score() {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"model":"guard","input":"hello"}"#,
        (200, answer("1e-28")),
    )
    .await;
    assert_success(&reply);
    let score = reply.body["results"][0]["category_scores"]["violence"]
        .as_f64()
        .expect("number");
    assert_ne!(score, 0.0, "a finite score is never rounded to zero");
    assert_eq!(score, 1e-28);
    let exact = parse_decimal_lexeme("1e-28").expect("decimal");
    assert!(
        reply.raw.contains(&format!(r#""violence":{exact}"#)),
        "the smallest score keeps its digits: {}",
        reply.raw
    );
}

fn assert_refused(reply: &Reply, status: StatusCode) {
    assert_eq!(reply.status, status, "{}", reply.raw);
    assert_eq!(error_of(reply)["type"], json!("invalid_request_error"));
    assert_eq!(call_id(reply), Some("caller-id"));
    assert!(reply.sent.is_empty(), "no provider request: {}", reply.sent);
}

async fn malformed_body() {
    let reply = call("", "/v1", "/v1/moderations", "{", (200, answer("0.5"))).await;
    assert_refused(&reply, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&reply)["code"], json!("400"));
}

async fn missing_input() {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"model":"guard"}"#,
        (200, answer("0.5")),
    )
    .await;
    assert_refused(&reply, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&reply)["code"], json!("400"));
}

async fn model_required() {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"input":"hello"}"#,
        (200, answer("0.5")),
    )
    .await;
    assert_refused(&reply, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&reply)["param"], json!("model"));
    assert_eq!(error_of(&reply)["code"], json!("moderation_model_required"));
}

async fn unknown_model() {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"model":"ghost","input":"hello"}"#,
        (200, answer("0.5")),
    )
    .await;
    assert_refused(&reply, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&reply)["code"], json!("400"));
    assert!(
        reply
            .raw
            .contains("no model_list entry serves the model ghost"),
        "{}",
        reply.raw
    );
}

async fn request_supplied_destination() {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"model":"guard","input":"hello","api_base":"http://evil.example/v1","api_key":"sk-evil"}"#,
        (200, answer("0.5")),
    )
    .await;
    assert_refused(&reply, StatusCode::BAD_REQUEST);
    assert!(reply.raw.contains("api_base"), "{}", reply.raw);
    assert!(!reply.raw.contains("evil.example"), "{}", reply.raw);
    assert!(!reply.raw.contains("sk-evil"), "{}", reply.raw);
}

async fn call_id_in_error_body() {
    let reply = call(
        "general_settings: {include_call_id_in_error_body: true}\n",
        "/v1",
        "/v1/moderations",
        r#"{"model":"guard","input":"hello"}"#,
        (400, String::from(PROVIDER_400)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert!(
        reply.raw.contains(r#""litellm_call_id":"caller-id""#),
        "{}",
        reply.raw
    );
}

async fn call_id_left_out_of_error_body() {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"model":"guard","input":"hello"}"#,
        (400, String::from(PROVIDER_400)),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert!(!reply.raw.contains("litellm_call_id"), "{}", reply.raw);
}

fn application_for(source: &str, environment: &BTreeMap<String, String>) -> Router {
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(source)
    .expect("config");
    let executor =
        chat_executor(&config, environment, StdRng::seed_from_u64(1)).expect("chat executor");
    application(
        &config,
        executor,
        Arc::new(AtomicBool::new(false)),
        &TaskTracker::new(),
    )
    .expect("application")
}

async fn auth_required() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let source = format!(
        "general_settings: {{master_key: oracle-master}}\nmodel_list:\n  - model_name: guard\n    litellm_params: {{model: openai/omni-moderation-latest, api_base: 'http://{address}/v1', api_key: oracle-key, timeout: 0.5}}\n"
    );
    let environment = BTreeMap::new();
    let unauthorized = exchange(
        application_for(&source, &environment),
        Request::post("/v1/moderations")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"model":"guard","input":"hello"}"#))
            .expect("request"),
        ready(String::new()),
    )
    .await;
    assert_eq!(unauthorized.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(unauthorized.raw, "Internal Server Error");
    let authorized = exchange(
        application_for(&source, &environment),
        request("/v1/moderations", r#"{"model":"guard","input":"hello"}"#),
        record_request(listener, (200, answer("0.5"))),
    )
    .await;
    assert_success(&authorized);
    assert!(
        authorized.sent.starts_with("POST /v1/moderations "),
        "{}",
        authorized.sent
    );
}

async fn body_limit() {
    let source = "general_settings:\n  max_request_size_mb: 1\nmodel_list:\n  - model_name: guard\n    litellm_params: {model: openai/omni-moderation-latest, api_base: 'http://127.0.0.1:1/v1', api_key: oracle-key, timeout: 0.2}\n";
    let oversized = vec![b'a'; 2 * 1024 * 1024];
    let reply = exchange(
        application_for(source, &BTreeMap::new()),
        Request::post("/v1/moderations")
            .header("content-type", "application/json")
            .header("content-length", oversized.len().to_string())
            .body(Body::from(oversized))
            .expect("request"),
        ready(String::new()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        reply.raw,
        r#"{"error":"Request size is too large. Max size is 1 MB"}"#
    );
}

async fn deployment_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let source = format!(
        "model_list:\n  - model_name: guard\n    litellm_params: {{model: openai/omni-moderation-latest, api_base: 'http://{address}/v1', api_key: oracle-key, timeout: 0.2}}\n"
    );
    let state = state_for(&config(&source));
    let provider = tokio::spawn(hang(listener));
    let started = Instant::now();
    let reply = exchange(
        routes(&state),
        request("/v1/moderations", r#"{"model":"guard","input":"hello"}"#),
        ready(String::new()),
    )
    .await;
    provider.abort();
    assert_eq!(reply.status, StatusCode::REQUEST_TIMEOUT, "{}", reply.raw);
    assert_eq!(error_of(&reply)["type"], Value::Null);
    assert_eq!(error_of(&reply)["param"], Value::Null);
    assert_eq!(error_of(&reply)["code"], json!("408"));
    assert_eq!(call_id(&reply), Some("caller-id"));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the deployment timeout bounds the call: {:?}",
        started.elapsed()
    );
}
