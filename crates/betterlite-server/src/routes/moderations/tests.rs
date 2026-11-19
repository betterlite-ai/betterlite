#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::str::from_utf8;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_openai::chat::deployment::ChatDeployment;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use serde::Deserialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
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

// every category appears in each map, and the score map carries the provider's lexeme
fn repeated(value: &str) -> String {
    CATEGORIES
        .iter()
        .map(|name| format!(r#""{name}":{value}"#))
        .collect::<Vec<_>>()
        .join(",")
}

// the provider answers every category, so the route has a full response to rewrite
fn answer(score: &str) -> String {
    format!(
        r#"{{"id":"modr-1","model":"omni-moderation-latest","results":[{{"categories":{{{}}},"category_applied_input_types":{{{}}},"category_scores":{{{}}},"flagged":false}}]}}"#,
        repeated("false"),
        repeated(r#"["text"]"#),
        repeated(score)
    )
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
        .map(|index| index.saturating_add(4))
}

fn content_length(head: &[u8]) -> usize {
    let text = from_utf8(head)
        .expect("the provider request head is UTF-8")
        .to_ascii_lowercase();
    text.lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

async fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut received = Vec::new();
    let mut buffer = [0_u8; 4096];
    while let Ok(read) = stream.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        received.extend_from_slice(&buffer[..read]);
        if let Some(end) = header_end(&received)
            && received.len() >= end.saturating_add(content_length(&received[..end]))
        {
            break;
        }
    }
    received
}

async fn record_requests(listener: TcpListener, replies: Vec<(u16, String)>) -> Vec<String> {
    // a retry waits 500 ms and then 1 s after the answer it repeats, so a scripted sequence needs the longer wait
    let accept_wait = if replies.len() > 1 {
        Duration::from_secs(2)
    } else {
        Duration::from_millis(800)
    };
    let mut sent = Vec::new();
    for (status, body) in replies {
        let Ok(Ok((mut stream, _))) = timeout(accept_wait, listener.accept()).await else {
            break;
        };
        let received = read_request(&mut stream).await;
        let head = format!(
            "HTTP/1.1 {status} X\r\ncontent-length: {}\r\ncontent-type: application/json\r\nconnection: close\r\nx-request-id: provider-call\r\n\r\n",
            body.len()
        );
        let _written = stream.write_all(format!("{head}{body}").as_bytes()).await;
        sent.push(String::from_utf8(received).expect("the provider request is UTF-8"));
    }
    sent
}

// the provider accepts and never answers, so only the deployment timeout ends the call
async fn hold(listener: TcpListener) -> Vec<String> {
    let Ok(Ok((mut stream, _))) = timeout(Duration::from_secs(2), listener.accept()).await else {
        return Vec::new();
    };
    let mut buffer = [0_u8; 1024];
    let _read = stream.read(&mut buffer).await;
    // the client closes the connection once its deployment timeout fires
    while let Ok(Ok(read)) = timeout(Duration::from_secs(2), stream.read(&mut buffer)).await {
        if read == 0 {
            break;
        }
    }
    Vec::new()
}

#[derive(Debug)]
struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    raw: Vec<u8>,
    sent: Vec<String>,
    // the deployment the provider request shows the route served, as the success headers report it
    served: Option<Served>,
}

// the two identity headers LiteLLM's success path reads off the served deployment
#[derive(Debug)]
struct Served {
    model_id: String,
    api_base: String,
}

// the client body the route answers, read through the keys this test checks; the provider's slash keys stay unknown
#[derive(Debug, Deserialize)]
struct AnswerBody {
    id: String,
    model: String,
    results: Vec<AnswerResult>,
}

#[derive(Debug, Deserialize)]
struct AnswerResult {
    categories: AnswerCategories,
    flagged: bool,
}

#[derive(Debug, Deserialize)]
struct AnswerCategories {
    harassment: bool,
    harassment_threatening: bool,
    illicit: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ErrorReply {
    error: ErrorFields,
}

#[derive(Debug, Deserialize)]
struct ErrorFields {
    message: String,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    param: Option<String>,
    code: String,
}

async fn exchange<Provider>(router: Router, request: Request<Body>, provider: Provider) -> Reply
where
    Provider: Future<Output = Vec<String>>,
{
    let (response, sent) = tokio::join!(router.oneshot(request), provider);
    let reply = response.expect("response");
    let status = reply.status();
    let headers = reply.headers().clone();
    let raw = to_bytes(reply.into_body(), 0x0010_0000)
        .await
        .expect("body")
        .to_vec();
    Reply {
        status,
        headers,
        raw,
        sent,
        served: None,
    }
}

async fn call(
    extra: &str,
    base_suffix: &str,
    path: &str,
    body: &str,
    replies: Vec<(u16, String)>,
) -> Reply {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let guard = format!("http://{address}{base_suffix}");
    let other = format!("http://{address}/other/v1");
    let state = state_with(&guard, &other, extra);
    let mut reply = exchange(
        routes(&state),
        request(path, body),
        record_requests(listener, replies),
    )
    .await;
    let served = served_deployment(&state.executor.deployments, &reply);
    reply.served = Some(served);
    reply
}

// the provider request's path names the deployment the route picked, so the expected headers follow from it
fn served_deployment(deployments: &[ChatDeployment], reply: &Reply) -> Served {
    let group = if reply
        .sent
        .first()
        .is_some_and(|sent| sent.starts_with("POST /other/"))
    {
        "other"
    } else {
        "guard"
    };
    let deployment = deployments
        .iter()
        .find(|deployment| deployment.model_group == group)
        .expect("the config has that model group");
    Served {
        model_id: deployment.model_id.clone(),
        api_base: deployment.api_base.clone(),
    }
}

fn body_text(reply: &Reply) -> &str {
    from_utf8(&reply.raw).expect("the route answer is UTF-8")
}

fn answer_body(reply: &Reply) -> AnswerBody {
    serde_json::from_slice(&reply.raw).expect("the route answers a moderation body")
}

fn error_details(reply: &Reply) -> ErrorFields {
    serde_json::from_slice::<ErrorReply>(&reply.raw)
        .expect("the route answers an error body")
        .error
}

fn header_text<'reply>(reply: &'reply Reply, name: &str) -> Option<&'reply str> {
    let value = reply.headers.get(name)?;
    value.to_str().ok()
}

fn call_id(reply: &Reply) -> Option<&str> {
    header_text(reply, "x-litellm-call-id")
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
    NoCooldown,
    ProviderRetryThenSuccess,
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
#[case::no_cooldown(Scenario::NoCooldown)]
#[case::provider_retry_then_success(Scenario::ProviderRetryThenSuccess)]
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
        Scenario::ConfiguredDefault => configured_default(r#"{"input":"hello"}"#).await,
        Scenario::ConfiguredDefaultOverridesBody => {
            configured_default(r#"{"model":"other","input":"hello"}"#).await;
        }
        Scenario::ExplicitOtherModel => explicit_other_model().await,
        Scenario::UnversionedPath => unversioned_path().await,
        Scenario::Provider400 => provider_error(400, PROVIDER_400).await,
        Scenario::Provider401 => provider_error(401, PROVIDER_401).await,
        Scenario::Provider429 => provider_error(429, PROVIDER_429).await,
        Scenario::Provider500 => provider_error(500, PROVIDER_500).await,
        Scenario::NoCooldown => no_cooldown().await,
        Scenario::ProviderRetryThenSuccess => provider_retry_then_success().await,
        Scenario::DecimalScore => decimal_score().await,
        Scenario::SmallestScore => smallest_score().await,
        Scenario::MalformedBody => malformed_body().await,
        Scenario::MissingInput => missing_input().await,
        Scenario::ModelRequired => model_required().await,
        Scenario::UnknownModel => unknown_model().await,
        Scenario::RequestSuppliedDestination => request_supplied_destination().await,
        Scenario::CallIdInErrorBody => call_id_body_case(true).await,
        Scenario::CallIdLeftOutOfErrorBody => call_id_body_case(false).await,
        Scenario::AuthRequired => auth_required().await,
        Scenario::BodyLimit => body_limit().await,
        Scenario::DeploymentTimeout => deployment_timeout().await,
    }
}

fn assert_success(reply: &Reply) {
    let text = body_text(reply);
    assert_eq!(reply.status, StatusCode::OK, "{text}");
    assert_eq!(header_text(reply, "content-type"), Some("application/json"));
    assert_eq!(call_id(reply), Some("caller-id"));
    assert_success_headers(reply);
    let body = answer_body(reply);
    assert_eq!(body.id, "modr-1");
    assert_eq!(body.model, "omni-moderation-latest");
    assert!(!body.results.first().expect("result").flagged);
}

// proxy_server.py's moderation handler hands get_custom_headers the served deployment's model id, its api base and the hidden params update_response_metadata wrote _response_ms and callback_duration_ms into
fn assert_success_headers(reply: &Reply) {
    let served = reply
        .served
        .as_ref()
        .expect("the caller named the served deployment");
    assert_eq!(
        header_text(reply, "x-litellm-model-id"),
        Some(served.model_id.as_str()),
        "x-litellm-model-id"
    );
    assert_eq!(
        header_text(reply, "x-litellm-model-api-base"),
        Some(served.api_base.as_str()),
        "x-litellm-model-api-base"
    );
    assert_eq!(header_text(reply, "x-litellm-key-spend"), Some("0.0"));
    // betterlite runs no callbacks, so update_response_metadata records the logging object's 0.0 callback duration
    assert_eq!(
        header_text(reply, "x-litellm-callback-duration-ms"),
        Some("0.0")
    );
    assert!(reply.headers.contains_key("x-litellm-version"));
    // the elapsed milliseconds reach custom_headers as hidden_params["_response_ms"] and print as a float
    let millis: f64 = header_text(reply, "x-litellm-response-duration-ms")
        .expect("x-litellm-response-duration-ms")
        .parse()
        .expect("the duration header is a number");
    assert!(millis.is_finite() && millis.is_sign_positive(), "{millis}");
    // the handler passes no cache key and no logging object, so the name and cost headers stay off
    for header in [
        "x-litellm-model-name",
        "x-litellm-model-group",
        "x-litellm-response-cost",
        "x-litellm-attempted-retries",
        "llm_provider-content-type",
    ] {
        assert!(
            !reply.headers.contains_key(header),
            "{header}: {:?}",
            reply.headers
        );
    }
}

// one successful moderation call on the versioned route, checked down to the bytes the provider received
async fn answered(extra: &str, path: &str, body: &str, score: &str, expected: &[&str]) -> Reply {
    let reply = call(extra, "/v1", path, body, vec![(200, answer(score))]).await;
    assert_success(&reply);
    let sent = reply.sent.first().expect("provider request");
    for fragment in expected {
        assert!(sent.contains(fragment), "{sent}");
    }
    reply
}

fn assert_sent(reply: &Reply, fragment: &str) {
    let sent = reply.sent.first().expect("provider request");
    assert!(sent.contains(fragment), "{sent}");
}

async fn input_case(body: &str, expected: &[&str]) {
    answered("", "/v1/moderations", body, "0.5", expected).await;
}

async fn array_input() {
    input_case(
        r#"{"model":"guard","input":["a","b"]}"#,
        &[r#"{"input":["a","b"],"model":"omni-moderation-latest"}"#],
    )
    .await;
}

async fn multimodal_input() {
    input_case(
        r#"{"model":"guard","input":[{"type":"image_url","image_url":{"url":"https://example.test/image.png"}},{"type":"text","text":"hello"}]}"#,
        &[
            r#"{"image_url":{"url":"https://example.test/image.png"},"type":"image_url"}"#,
            r#"{"text":"hello","type":"text"}"#,
        ],
    )
    .await;
}

async fn string_input() {
    let reply = answered(
        "",
        "/v1/moderations",
        r#"{"model":"guard","input":"hello"}"#,
        "0.0001",
        &[r#"{"input":"hello","model":"omni-moderation-latest"}"#],
    )
    .await;
    let body = answer_body(&reply);
    let categories = &body.results.first().expect("result").categories;
    assert!(!categories.harassment);
    assert!(!categories.harassment_threatening);
    assert_eq!(categories.illicit, Some(false));
    // LiteLLM dumps the SDK model, which keeps the provider's slash key beside its field name
    let text = body_text(&reply);
    assert!(text.contains(r#""harassment_threatening":false"#), "{text}");
    assert!(text.contains(r#""harassment/threatening":false"#), "{text}");
    assert!(text.contains(r#""self_harm":false"#), "{text}");
    assert!(text.contains(r#""self-harm":false"#), "{text}");
    assert!(text.contains(r#""violence_graphic":0.0001"#), "{text}");
    assert!(text.contains(r#""violence/graphic":0.0001"#), "{text}");
}

async fn configured_default(body: &str) {
    answered(
        "general_settings: {moderation_model: guard}\n",
        "/v1/moderations",
        body,
        "0.5",
        &[
            r#""model":"omni-moderation-latest""#,
            "POST /v1/moderations ",
        ],
    )
    .await;
}

async fn explicit_other_model() {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"model":"other","input":"hello"}"#,
        vec![(200, answer("0.5"))],
    )
    .await;
    assert_success(&reply);
    assert_sent(&reply, r#""model":"other-moderation""#);
    assert_sent(&reply, "POST /other/v1/moderations ");
}

async fn unversioned_path() {
    let reply = call(
        "",
        "",
        "/moderations",
        r#"{"model":"guard","input":"hello"}"#,
        vec![(200, answer("0.5"))],
    )
    .await;
    assert_success(&reply);
    assert_sent(&reply, "POST /moderations ");
}

async fn refused_case(body: &str) -> Reply {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        body,
        vec![(200, answer("0.5"))],
    )
    .await;
    assert_refused(&reply, StatusCode::BAD_REQUEST);
    reply
}

async fn malformed_body() {
    let reply = refused_case("{").await;
    assert_eq!(error_details(&reply).code, "400");
}

async fn missing_input() {
    let reply = refused_case(r#"{"model":"guard"}"#).await;
    assert_eq!(error_details(&reply).code, "400");
}

async fn model_required() {
    let reply = refused_case(r#"{"input":"hello"}"#).await;
    let error = error_details(&reply);
    assert_eq!(error.param.as_deref(), Some("model"));
    assert_eq!(error.code, "moderation_model_required");
}

async fn unknown_model() {
    let reply = refused_case(r#"{"model":"ghost","input":"hello"}"#).await;
    let error = error_details(&reply);
    assert!(
        error
            .message
            .contains("no model_list entry serves the model ghost"),
        "{}",
        body_text(&reply)
    );
}

async fn request_supplied_destination() {
    let reply = refused_case(
        r#"{"model":"guard","input":"hello","api_base":"http://evil.example/v1","api_key":"sk-evil"}"#,
    )
    .await;
    let text = body_text(&reply);
    assert!(text.contains("api_base"), "{text}");
    assert!(!text.contains("evil.example"), "{text}");
    assert!(!text.contains("sk-evil"), "{text}");
}

async fn call_id_body_case(present: bool) {
    let extra = if present {
        "general_settings: {include_call_id_in_error_body: true}\n"
    } else {
        ""
    };
    let reply = call(
        extra,
        "/v1",
        "/v1/moderations",
        r#"{"model":"guard","input":"hello"}"#,
        vec![(400, String::from(PROVIDER_400))],
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body_text(&reply).contains(r#""litellm_call_id":"caller-id""#),
        present,
        "{}",
        body_text(&reply)
    );
}

async fn provider_error(status: u16, body: &str) {
    let attempts = if matches!(status, 408 | 409 | 429 | 500..=599) {
        3
    } else {
        1
    };
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"model":"guard","input":"hello"}"#,
        vec![(status, body.to_owned()); attempts],
    )
    .await;
    assert_eq!(reply.sent.len(), attempts, "the SDK's retry count");
    assert_provider_error(status, &reply);
}

// LiteLLM's router puts a deployment in cooldown for a provider error it does not retry; betterlite keeps it in service
async fn no_cooldown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let guard = format!("http://{address}/v1");
    let other = format!("http://{address}/other/v1");
    let state = state_with(&guard, &other, "");
    let mut provider = JoinSet::new();
    provider.spawn(record_requests(
        listener,
        vec![(401, String::from(PROVIDER_401)), (200, answer("0.5"))],
    ));
    let first = routes(&state)
        .oneshot(request(
            "/v1/moderations",
            r#"{"model":"guard","input":"hello"}"#,
        ))
        .await
        .expect("first answer");
    let second = routes(&state)
        .oneshot(request(
            "/v1/moderations",
            r#"{"model":"guard","input":"hello"}"#,
        ))
        .await
        .expect("second answer");
    let sent = provider
        .join_next()
        .await
        .expect("provider task")
        .expect("provider result");
    assert_eq!(first.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(sent.len(), 2, "the second request reached the provider");
}

// the moderation except answers litellm_call_id_headers alone
fn assert_failure_headers(reply: &Reply) {
    assert!(
        !reply.headers.contains_key("llm_provider-content-type"),
        "{:?}",
        reply.headers
    );
    assert!(!reply.headers.contains_key("x-litellm-timeout"));
    assert!(!reply.headers.contains_key("x-litellm-response-duration-ms"));
}

// the provider body's type and param for each status the stubbed provider answers
const fn provider_fields(status: u16) -> Option<(Option<&'static str>, Option<&'static str>)> {
    match status {
        400 => Some((Some("invalid_request_error"), Some("input"))),
        401 => Some((Some("invalid_request_error"), None)),
        429 => Some((Some("tokens"), None)),
        500 => Some((None, None)),
        _ => None,
    }
}

fn assert_provider_error(status: u16, reply: &Reply) {
    let text = body_text(reply);
    assert_eq!(reply.status.as_u16(), status, "{text}");
    let error = error_details(reply);
    assert_eq!(error.code, status.to_string(), "{text}");
    assert_eq!(call_id(reply), Some("caller-id"));
    assert_failure_headers(reply);
    let (kind, param) = provider_fields(status).expect("a stubbed provider status");
    assert_eq!(error.kind.as_deref(), kind, "{text}");
    assert_eq!(error.param.as_deref(), param, "{text}");
    // betterlite writes its own message instead of copying the SDK exception's, which repeats the provider body
    assert_eq!(
        error.message,
        format!("the provider answered with status {status}"),
        "{text}"
    );
    // an absent type or param reaches the client as an explicit null
    assert_eq!(text.contains(r#""type":null"#), kind.is_none(), "{text}");
    assert_eq!(text.contains(r#""param":null"#), param.is_none(), "{text}");
}

async fn provider_retry_then_success() {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"model":"guard","input":"hello"}"#,
        vec![(429, String::from(PROVIDER_429)), (200, answer("0.5"))],
    )
    .await;
    assert_success(&reply);
    assert_eq!(reply.sent.len(), 2, "the SDK retries the 429 once");
}

async fn decimal_score() {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"model":"guard","input":"hello"}"#,
        vec![(200, answer("1e-29"))],
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::BAD_GATEWAY,
        "{}",
        body_text(&reply)
    );
    assert_eq!(
        body_text(&reply),
        r#"{"error":{"message":"the category_scores value for harassment is not exactly representable as a Decimal: Scale exceeds the maximum precision allowed: 29 > 28","type":"bad_gateway","param":"category_scores","code":"decimal_out_of_range"}}"#
    );
}

async fn smallest_score() {
    let reply = call(
        "",
        "/v1",
        "/v1/moderations",
        r#"{"model":"guard","input":"hello"}"#,
        vec![(200, answer("1e-28"))],
    )
    .await;
    assert_success(&reply);
    let exact = parse_decimal_lexeme("1e-28").expect("decimal");
    let text = body_text(&reply);
    assert!(
        text.contains(&format!(r#""violence":{exact}"#)),
        "the smallest score keeps its digits: {text}"
    );
}

fn assert_refused(reply: &Reply, status: StatusCode) {
    let text = body_text(reply);
    assert_eq!(reply.status, status, "{text}");
    assert_eq!(
        error_details(reply).kind.as_deref(),
        Some("invalid_request_error"),
        "{text}"
    );
    assert_eq!(call_id(reply), Some("caller-id"));
    assert!(
        reply.sent.is_empty(),
        "no provider request: {:?}",
        reply.sent
    );
}

fn application_for(
    source: &str,
    environment: &BTreeMap<String, String>,
) -> (Router, Vec<ChatDeployment>) {
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(source)
    .expect("config");
    let executor =
        chat_executor(&config, environment, StdRng::seed_from_u64(1)).expect("chat executor");
    let deployments = executor.deployments.clone();
    let app = application(
        &config,
        executor,
        Arc::new(AtomicBool::new(false)),
        &TaskTracker::new(),
    )
    .expect("application");
    (app, deployments)
}

async fn auth_required() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let source = format!(
        "general_settings: {{master_key: oracle-master}}\nmodel_list:\n  - model_name: guard\n    litellm_params: {{model: openai/omni-moderation-latest, api_base: 'http://{address}/v1', api_key: oracle-key, timeout: 0.5}}\n"
    );
    let environment = BTreeMap::new();
    let (unauthorized_app, _deployments) = application_for(&source, &environment);
    let unauthorized = exchange(
        unauthorized_app,
        Request::post("/v1/moderations")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"model":"guard","input":"hello"}"#))
            .expect("request"),
        ready(Vec::new()),
    )
    .await;
    assert_eq!(unauthorized.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body_text(&unauthorized), "Internal Server Error");
    let (authorized_app, deployments) = application_for(&source, &environment);
    let mut authorized = exchange(
        authorized_app,
        request("/v1/moderations", r#"{"model":"guard","input":"hello"}"#),
        record_requests(listener, vec![(200, answer("0.5"))]),
    )
    .await;
    let served = served_deployment(&deployments, &authorized);
    authorized.served = Some(served);
    assert_success(&authorized);
    assert!(
        authorized
            .sent
            .first()
            .is_some_and(|sent| sent.starts_with("POST /v1/moderations ")),
        "{:?}",
        authorized.sent
    );
}

async fn body_limit() {
    let source = "general_settings:\n  max_request_size_mb: 1\nmodel_list:\n  - model_name: guard\n    litellm_params: {model: openai/omni-moderation-latest, api_base: 'http://127.0.0.1:1/v1', api_key: oracle-key, timeout: 0.2}\n";
    let oversized = vec![b'a'; 0x0020_0000];
    let (app, _deployments) = application_for(source, &BTreeMap::new());
    let reply = exchange(
        app,
        Request::post("/v1/moderations")
            .header("content-type", "application/json")
            .header("content-length", oversized.len().to_string())
            .body(Body::from(oversized))
            .expect("request"),
        ready(Vec::new()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        body_text(&reply),
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
    let started = Instant::now();
    let reply = exchange(
        routes(&state),
        request("/v1/moderations", r#"{"model":"guard","input":"hello"}"#),
        hold(listener),
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::REQUEST_TIMEOUT,
        "{}",
        body_text(&reply)
    );
    let error = error_details(&reply);
    assert_eq!(error.kind, None);
    assert_eq!(error.param, None);
    assert_eq!(error.code, "408");
    assert_eq!(call_id(&reply), Some("caller-id"));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the deployment timeout bounds the call: {:?}",
        started.elapsed()
    );
}
