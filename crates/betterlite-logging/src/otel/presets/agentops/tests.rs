#![cfg(test)]

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use http::{Method, StatusCode};
use opentelemetry::InstrumentationScope;
use opentelemetry::trace::{SpanContext, SpanId, SpanKind, Status};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanExporter as _, SpanLinks};
use rstest::rstest;
use secrecy::SecretString;
use tokio::sync::OnceCell;
use tokio::time::sleep;

use super::{
    AGENTOPS_EXPORTER_KIND, AgentopsAuthClient, AgentopsPreset, TokenRequest,
    agentops_exporter_factory,
};
use crate::otel::config::destination::SpanScope;
use crate::otel::config::{CaptureMessageContent, ExporterOwner, ExporterSpec, Settings};
use crate::otel::plumbing::exporters::{ExporterFactory, HttpClients};
use crate::otel::presets::base::Preset as _;

const AUTH_URL: &str = "https://api.agentops.ai/v3/auth/token";
const TOKEN: &str = "jwt-token-1";

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn base_settings() -> Settings {
    Settings {
        exporters: vec![ExporterSpec {
            kind: "otlp_http".to_owned(),
            endpoint: Some("https://collector.example/v1/traces".to_owned()),
            traces_endpoint: None,
            headers: Some("x-collector=1".to_owned()),
            owner: None,
            options: None,
            use_simple_processor: None,
            requires_headers: false,
        }],
        service_name: "litellm".to_owned(),
        deployment_environment: Some("production".to_owned()),
        enable_metrics: false,
        enable_events: false,
        capture_message_content: CaptureMessageContent::NoContent,
        legacy_compat: false,
        langfuse_span_scope: SpanScope::Full,
        excluded_services: BTreeSet::new(),
        mapper_names: vec!["genai".to_owned(), "openinference".to_owned()],
        resource_attributes: BTreeMap::from([
            ("service.name".to_owned(), "proxy".to_owned()),
            ("telemetry.sdk.name".to_owned(), "opentelemetry".to_owned()),
            ("deployment.environment".to_owned(), "staging".to_owned()),
        ]),
        baggage_promoted_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        baggage_team_metadata_keys: Vec::new(),
    }
}

fn build(variables: &[(&str, &str)]) -> Settings {
    AgentopsPreset
        .build(&environment(variables), base_settings(), false)
        .expect("the agentops preset cannot fail")
}

#[test]
fn preset_appends_the_agentops_exporter_and_overrides_the_resource_attributes() {
    let settings = build(&[
        ("AGENTOPS_API_KEY", "key-1"),
        ("AGENTOPS_SERVICE_NAME", "my-service"),
        ("AGENTOPS_ENVIRONMENT", "production"),
    ]);
    assert_eq!(settings.exporters.len(), 2);
    assert_eq!(
        settings.exporters[1],
        ExporterSpec {
            kind: "agentops".to_owned(),
            endpoint: Some("https://otlp.agentops.ai/v1/traces".to_owned()),
            traces_endpoint: None,
            headers: None,
            owner: Some(ExporterOwner::Agentops),
            options: Some(BTreeMap::from([("api_key".to_owned(), "key-1".to_owned())])),
            use_simple_processor: None,
            requires_headers: false,
        }
    );
    assert_eq!(
        settings.resource_attributes,
        BTreeMap::from([
            ("deployment.environment".to_owned(), "production".to_owned()),
            ("service.name".to_owned(), "my-service".to_owned()),
            ("telemetry.sdk.name".to_owned(), "agentops".to_owned()),
        ])
    );
    assert_eq!(settings.mapper_names, ["genai", "openinference"]);
    assert_eq!(
        settings.deployment_environment,
        Some("production".to_owned())
    );
}

#[rstest]
#[case::unset(&[], "agentops")]
#[case::empty(&[("AGENTOPS_SERVICE_NAME", "")], "")]
#[case::given(&[("AGENTOPS_SERVICE_NAME", "proxy")], "proxy")]
fn preset_reads_the_service_name(#[case] variables: &[(&str, &str)], #[case] expected: &str) {
    let settings = build(variables);
    assert_eq!(
        settings.resource_attributes.get("service.name"),
        Some(&expected.to_owned())
    );
}

#[rstest]
#[case::empty(&[("AGENTOPS_ENVIRONMENT", "")], None)]
#[case::unset(&[], None)]
#[case::given(&[("AGENTOPS_ENVIRONMENT", "qa")], Some("qa"))]
fn preset_adds_the_deployment_environment_only_when_given(
    #[case] variables: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let mut expected_attributes = BTreeMap::from([
        ("service.name".to_owned(), "agentops".to_owned()),
        ("telemetry.sdk.name".to_owned(), "agentops".to_owned()),
    ]);
    if let Some(name) = expected {
        expected_attributes.insert("deployment.environment".to_owned(), name.to_owned());
    }
    assert_eq!(build(variables).resource_attributes, expected_attributes);
}

#[rstest]
#[case::missing(&[], None)]
#[case::empty(&[("AGENTOPS_API_KEY", "")], None)]
#[case::space(&[("AGENTOPS_API_KEY", " ")], Some(" "))]
#[case::zero(&[("AGENTOPS_API_KEY", "0")], Some("0"))]
#[case::given(&[("AGENTOPS_API_KEY", "key-1")], Some("key-1"))]
fn preset_puts_the_api_key_in_the_options_only_when_present(
    #[case] variables: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        build(variables).exporters[1].options,
        expected.map(|key| BTreeMap::from([("api_key".to_owned(), key.to_owned())]))
    );
}

#[derive(Debug, Clone, Copy)]
enum AuthAnswer {
    Token(&'static str),
    Status(StatusCode),
    Body(&'static str),
    DelayedToken(&'static str),
    Stall,
}

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: Method,
    uri: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl RecordedRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find_map(|(key, value)| (key == name).then_some(value.as_str()))
    }
}

#[derive(Debug)]
struct FakeClient {
    answer: AuthAnswer,
    requests: Mutex<Vec<RecordedRequest>>,
    auth_posts: AtomicUsize,
}

impl FakeClient {
    fn new(answer: AuthAnswer) -> Self {
        Self {
            answer,
            requests: Mutex::new(Vec::new()),
            auth_posts: AtomicUsize::new(0),
        }
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.requests
            .lock()
            .expect("the recorder lock is held")
            .clone()
    }

    fn exports(&self) -> Vec<RecordedRequest> {
        self.requests()
            .into_iter()
            .filter(|request| request.uri != AUTH_URL)
            .collect()
    }

    fn auth_posts(&self) -> usize {
        self.auth_posts.load(Ordering::SeqCst)
    }

    fn record(&self, request: &Request<Bytes>) {
        let mut headers = Vec::new();
        for (name, value) in request.headers() {
            headers.push((
                name.to_string(),
                value.to_str().unwrap_or_default().to_owned(),
            ));
        }
        self.requests
            .lock()
            .expect("the recorder lock is not poisoned")
            .push(RecordedRequest {
                method: request.method().clone(),
                uri: request.uri().to_string(),
                headers,
                body: request.body().to_vec(),
            });
    }

    async fn answer_auth(&self) -> Result<Response<Bytes>, HttpError> {
        match self.answer {
            AuthAnswer::Token(token) => Ok(token_response(token)),
            AuthAnswer::Status(status) => {
                Ok(Response::builder().status(status).body(Bytes::new())?)
            }
            AuthAnswer::Body(body) => Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Bytes::from_static(body.as_bytes()))?),
            AuthAnswer::DelayedToken(token) => {
                sleep(Duration::from_millis(20)).await;
                Ok(token_response(token))
            }
            AuthAnswer::Stall => {
                sleep(Duration::from_secs(11)).await;
                Ok(token_response("late-token"))
            }
        }
    }
}

#[async_trait]
impl HttpClient for FakeClient {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        self.record(&request);
        if request.uri() == AUTH_URL {
            self.auth_posts.fetch_add(1, Ordering::SeqCst);
            return self.answer_auth().await;
        }
        Ok(Response::new(Bytes::new()))
    }
}

fn token_response(token: &str) -> Response<Bytes> {
    let body = format!(r#"{{"token":"{token}"}}"#);
    Response::new(Bytes::from(body))
}

fn clients(fake: &Arc<FakeClient>) -> HttpClients {
    let concrete = Arc::clone(fake);
    let shared: Arc<dyn HttpClient> = concrete;
    HttpClients {
        traces: Arc::clone(&shared),
        metrics: Arc::clone(&shared),
        logs: shared,
    }
}

fn spec(endpoint: Option<&str>) -> ExporterSpec {
    ExporterSpec {
        kind: AGENTOPS_EXPORTER_KIND.to_owned(),
        endpoint: endpoint.map(str::to_owned),
        traces_endpoint: None,
        headers: None,
        owner: Some(ExporterOwner::Agentops),
        options: Some(BTreeMap::from([("api_key".to_owned(), "key-1".to_owned())])),
        use_simple_processor: None,
        requires_headers: false,
    }
}

fn auth_client(fake: &Arc<FakeClient>, api_key: Option<&str>) -> AgentopsAuthClient {
    let concrete = Arc::clone(fake);
    let inner: Arc<dyn HttpClient> = concrete;
    AgentopsAuthClient {
        inner,
        api_key: api_key.map(SecretString::from),
        token: OnceCell::new(),
    }
}

fn export_request() -> Request<Bytes> {
    Request::post("https://collector.example/custom/traces")
        .body(Bytes::from_static(b"spans"))
        .expect("the export request builds")
}

fn span() -> SpanData {
    SpanData {
        span_context: SpanContext::empty_context(),
        parent_span_id: SpanId::INVALID,
        parent_span_is_remote: false,
        span_kind: SpanKind::Internal,
        name: Cow::Borrowed("test"),
        start_time: SystemTime::UNIX_EPOCH,
        end_time: SystemTime::UNIX_EPOCH,
        attributes: Vec::new(),
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: Status::Unset,
        instrumentation_scope: InstrumentationScope::default(),
    }
}

#[tokio::test]
async fn factory_posts_the_key_once_and_carries_the_bearer_on_every_export() {
    let fake = Arc::new(FakeClient::new(AuthAnswer::Token(TOKEN)));
    let factory: ExporterFactory = agentops_exporter_factory(&clients(&fake));
    let sink = factory(&spec(Some("https://collector.example/custom/traces")))
        .expect("the agentops exporter builds");
    sink.export(vec![span()])
        .await
        .expect("the first export runs");
    sink.export(vec![span()])
        .await
        .expect("the second export runs");

    let requests = fake.requests();
    assert_eq!(requests.len(), 3);
    let auth = &requests[0];
    assert_eq!(auth.method, Method::POST);
    assert_eq!(auth.uri, AUTH_URL);
    assert_eq!(auth.header("content-type"), Some("application/json"));
    assert_eq!(auth.header("connection"), Some("keep-alive"));
    assert_eq!(
        auth.body,
        serde_json::to_vec(&TokenRequest { api_key: "key-1" }).expect("the body is JSON")
    );
    let bearer = format!("Bearer {TOKEN}");
    let exports = fake.exports();
    assert_eq!(exports.len(), 2);
    assert!(exports.iter().all(|export| export.method == Method::POST));
    assert!(
        exports
            .iter()
            .all(|export| export.uri == "https://collector.example/custom/traces")
    );
    assert!(
        exports
            .iter()
            .all(|export| export.header("authorization") == Some(bearer.as_str()))
    );
    assert_eq!(fake.auth_posts(), 1);
}

#[tokio::test]
async fn factory_without_a_key_never_posts_a_token_request() {
    let fake = Arc::new(FakeClient::new(AuthAnswer::Token(TOKEN)));
    let client = auth_client(&fake, None);
    client
        .send_bytes(export_request())
        .await
        .expect("the export is sent");
    client
        .send_bytes(export_request())
        .await
        .expect("the export is sent");
    assert_eq!(fake.auth_posts(), 0);
    assert!(
        fake.exports()
            .iter()
            .all(|request| request.header("authorization").is_none())
    );
}

#[rstest]
#[case::status(AuthAnswer::Status(StatusCode::INTERNAL_SERVER_ERROR))]
#[case::not_json(AuthAnswer::Body("not json"))]
#[case::empty_token(AuthAnswer::Body(r#"{"token":""}"#))]
#[case::token_not_a_string(AuthAnswer::Body(r#"{"token":42}"#))]
#[case::missing_token(AuthAnswer::Body("{}"))]
#[tokio::test]
async fn a_failed_token_fetch_sends_without_the_header_and_is_not_retried(
    #[case] answer: AuthAnswer,
) {
    let fake = Arc::new(FakeClient::new(answer));
    let client = auth_client(&fake, Some("key-1"));
    client
        .send_bytes(export_request())
        .await
        .expect("the export is still sent");
    client
        .send_bytes(export_request())
        .await
        .expect("the export is still sent");
    assert_eq!(fake.auth_posts(), 1);
    assert!(
        fake.exports()
            .iter()
            .all(|request| request.header("authorization").is_none())
    );
}

#[tokio::test]
async fn concurrent_first_exports_share_one_token_fetch() {
    let fake = Arc::new(FakeClient::new(AuthAnswer::DelayedToken(TOKEN)));
    let client = Arc::new(auth_client(&fake, Some("key-1")));
    let (first, second) = tokio::join!(
        client.send_bytes(export_request()),
        client.send_bytes(export_request())
    );
    first.expect("the first export is sent");
    second.expect("the second export is sent");
    assert_eq!(fake.auth_posts(), 1);
    let bearer = format!("Bearer {TOKEN}");
    assert!(
        fake.exports()
            .iter()
            .all(|request| request.header("authorization") == Some(bearer.as_str()))
    );
}

#[tokio::test(start_paused = true)]
async fn a_stalled_token_fetch_times_out_and_is_not_retried() {
    let fake = Arc::new(FakeClient::new(AuthAnswer::Stall));
    let client = auth_client(&fake, Some("key-1"));
    client
        .send_bytes(export_request())
        .await
        .expect("the export is sent after the timeout");
    client
        .send_bytes(export_request())
        .await
        .expect("the export is sent");
    assert_eq!(fake.auth_posts(), 1);
    assert!(
        fake.exports()
            .iter()
            .all(|request| request.header("authorization").is_none())
    );
}

#[test]
fn the_factory_records_the_expected_kind() {
    assert_eq!(AGENTOPS_EXPORTER_KIND, "agentops");
}
