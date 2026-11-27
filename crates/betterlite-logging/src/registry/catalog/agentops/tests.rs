#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::Resource;
use rstest::rstest;

use super::{CALLBACK_NAME, build, connect_agentops, legacy_otel};
use crate::callback_logger::CallbackLogger;
use crate::otel::callbacks::agentops::transport::{
    AuthFailure, AuthPost, AuthResponse, AuthTransport,
};
use crate::otel::callbacks::agentops::{
    AUTH_ENDPOINT, AUTH_TIMEOUT, AgentOpsCallback, AgentOpsConfig, ENDPOINT,
};
use crate::otel::legacy_policy::config::{Exporter, OTLP_HTTP_EXPORTER};
use crate::otel::logger_runtime::tests::support::{Fixture, fixture};
use crate::otel::plumbing::exporters::{ExporterFactories, HttpClients};
use crate::otel::presets::agentops::{AGENTOPS_EXPORTER_KIND, agentops_exporter_factory};
use crate::registry::catalog::LoggerCatalog;
use crate::registry::catalog::otel::shared::SharedOtel;
use crate::registry::catalog::otel::tests::support::{catalog, catalog_inputs};
use crate::registry::error::RegistryError;

const CALLBACKS: &str = "litellm_settings:\n  callbacks: [agentops]\n";
const SUCCESS: &str = "litellm_settings:\n  success_callback: [agentops]\n";
const FAILURE: &str = "litellm_settings:\n  failure_callback: [agentops]\n";
const UNCONFIGURED: &str = "";
const API_KEY: (&str, &str) = ("AGENTOPS_API_KEY", "agentops-key");
const V2_ON: (&str, &str) = ("LITELLM_OTEL_V2", "true");
const V2_UNREADABLE: (&str, &str) = ("LITELLM_OTEL_V2", "maybe");
const TOKEN_BODY: &str = r#"{"token": "jwt-1", "project_id": "project-1"}"#;
const PROJECT_ONLY_BODY: &str = r#"{"project_id": "project-1"}"#;

struct FakeTransport {
    requests: Mutex<Vec<AuthPost>>,
    status: u16,
    body: &'static str,
}

impl FakeTransport {
    fn answering(status: u16, body: &'static str) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            status,
            body,
        }
    }

    fn token() -> Self {
        Self::answering(200, TOKEN_BODY)
    }

    fn requests(&self) -> Vec<AuthPost> {
        self.requests
            .lock()
            .expect("the captured requests lock")
            .clone()
    }
}

#[async_trait]
impl AuthTransport for FakeTransport {
    async fn post(&self, request: AuthPost) -> Result<AuthResponse, AuthFailure> {
        self.requests
            .lock()
            .expect("the captured requests lock")
            .push(request);
        Ok(AuthResponse {
            status: self.status,
            body: self.body.as_bytes().to_vec(),
        })
    }
}

struct NamedLogger {
    name: String,
}

impl CallbackLogger for NamedLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug)]
struct NoClient;

#[async_trait]
impl HttpClient for NoClient {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(Box::from("the agentops catalog tests never export"))
    }
}

fn http_clients() -> HttpClients {
    let client: Arc<dyn HttpClient> = Arc::new(NoClient);
    HttpClients {
        traces: Arc::clone(&client),
        metrics: Arc::clone(&client),
        logs: client,
    }
}

// the proxy runtime registers the preset's exporter kind; the fixture gets the same registration here
fn v2_runtime() -> Fixture {
    let mut fixture = fixture();
    let clients = http_clients();
    let custom = BTreeMap::from([(
        AGENTOPS_EXPORTER_KIND.to_owned(),
        agentops_exporter_factory(&clients),
    )]);
    fixture.runtime.factories = Arc::new(ExporterFactories::new(clients, custom));
    fixture
}

fn idle_callback() -> AgentOpsCallback {
    AgentOpsCallback {
        config: AgentOpsConfig::default(),
        auth: None,
    }
}

fn attributes(resource: &Resource) -> BTreeMap<String, String> {
    resource
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), value.as_str().into_owned()))
        .collect()
}

// the startup step posts the API key the way the AgentOps constructor does (agentops.py 56-65)
#[tokio::test]
async fn the_startup_step_fetches_the_token_with_the_configured_api_key() {
    let inputs = catalog_inputs(CALLBACKS, &[API_KEY]);
    let transport = FakeTransport::token();
    let callback = connect_agentops(&inputs, &transport).await;
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    let request = requests.first().expect("one token request");
    assert_eq!(request.url, AUTH_ENDPOINT);
    assert_eq!(request.timeout, AUTH_TIMEOUT);
    let auth = callback.auth.expect("the token response is kept");
    assert!(auth.token.is_some());
    assert_eq!(auth.project_id.as_deref(), Some("project-1"));
}

// litellm_settings.callbacks, success_callback and failure_callback all build the logger
#[rstest]
#[case::callbacks(CALLBACKS)]
#[case::success(SUCCESS)]
#[case::failure(FAILURE)]
#[tokio::test]
async fn each_callback_list_starts_the_startup_fetch(#[case] yaml: &str) {
    let inputs = catalog_inputs(yaml, &[API_KEY]);
    let transport = FakeTransport::token();
    let callback = connect_agentops(&inputs, &transport).await;
    assert_eq!(transport.requests().len(), 1);
    assert!(callback.auth.is_some());
}

// a name the config never lists builds no logger, so its startup step fetches nothing
#[tokio::test]
async fn an_unconfigured_catalog_starts_no_fetch() {
    let inputs = catalog_inputs(UNCONFIGURED, &[API_KEY]);
    let transport = FakeTransport::token();
    let callback = connect_agentops(&inputs, &transport).await;
    assert!(transport.requests().is_empty());
    assert!(callback.auth.is_none());
}

#[tokio::test]
async fn without_an_api_key_the_startup_step_sends_no_request() {
    let inputs = catalog_inputs(CALLBACKS, &[]);
    let transport = FakeTransport::token();
    let callback = connect_agentops(&inputs, &transport).await;
    assert!(transport.requests().is_empty());
    assert!(callback.auth.is_none());
}

// the startup step answers before the catalog resolves a name, so the arm builds from its result
#[tokio::test]
async fn the_catalog_builds_the_connected_logger_once_and_reuses_it() {
    let inputs = catalog_inputs(CALLBACKS, &[API_KEY]);
    let transport = FakeTransport::token();
    let callback = connect_agentops(&inputs, &transport).await;
    let catalog = LoggerCatalog::new(inputs).with_agentops(callback);
    let first = catalog
        .resolve_logger(CALLBACK_NAME)
        .expect("the name is available")
        .expect("the legacy logger builds");
    assert_eq!(first.callback_name(), CALLBACK_NAME);
    let second = catalog
        .resolve_logger(CALLBACK_NAME)
        .expect("the name is available")
        .expect("the second resolve reuses it");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(catalog.in_memory_loggers().len(), 1);
    assert_eq!(transport.requests().len(), 1);
}

#[test]
fn a_catalog_without_a_startup_connection_answers_unavailable() {
    let catalog = catalog(CALLBACKS, &[API_KEY]);
    assert!(matches!(
        catalog.resolve_logger(CALLBACK_NAME),
        Err(RegistryError::UnavailableAdapter { name }) if name == CALLBACK_NAME
    ));
    assert!(catalog.in_memory_loggers().is_empty());
}

// a key or team callback naming agentops never reached the startup step (docs/deviations/B10008.yaml)
#[tokio::test]
async fn a_key_callback_without_a_configured_agentops_answers_unavailable() {
    let inputs = catalog_inputs(UNCONFIGURED, &[API_KEY]);
    let transport = FakeTransport::token();
    let callback = connect_agentops(&inputs, &transport).await;
    assert!(transport.requests().is_empty());
    let catalog = LoggerCatalog::new(inputs).with_agentops(callback);
    assert!(matches!(
        catalog.resolve_logger(CALLBACK_NAME),
        Err(RegistryError::UnavailableAdapter { name }) if name == CALLBACK_NAME
    ));
}

// the token becomes the exporter header (agentops.py 67)
#[tokio::test]
async fn the_legacy_logger_carries_the_bearer_header() {
    let inputs = catalog_inputs(CALLBACKS, &[API_KEY]);
    let callback = connect_agentops(&inputs, &FakeTransport::token()).await;
    let shared = SharedOtel::new(&inputs);
    let otel = legacy_otel(&inputs, &shared, &callback).expect("the provider builds");
    assert_eq!(
        otel.config.exporter,
        Exporter::Named(OTLP_HTTP_EXPORTER.to_owned())
    );
    assert_eq!(otel.config.endpoint.as_deref(), Some(ENDPOINT));
    assert_eq!(
        otel.config.headers.as_deref(),
        Some("Authorization=Bearer jwt-1")
    );
    assert_eq!(otel.callback_name.as_deref(), Some(CALLBACK_NAME));
}

// the token response's project id becomes a resource attribute (agentops.py 81-82)
#[tokio::test]
async fn the_legacy_resource_carries_the_project_id() {
    let inputs = catalog_inputs(CALLBACKS, &[API_KEY]);
    let callback = connect_agentops(&inputs, &FakeTransport::token()).await;
    let shared = SharedOtel::new(&inputs);
    let otel = legacy_otel(&inputs, &shared, &callback).expect("the provider builds");
    let resource = attributes(&otel.resource);
    assert_eq!(
        resource.get("project.id").map(String::as_str),
        Some("project-1")
    );
    assert_eq!(
        resource.get("service.name").map(String::as_str),
        Some("agentops")
    );
    assert_eq!(
        resource.get("deployment.environment").map(String::as_str),
        Some("production")
    );
    assert_eq!(
        resource.get("telemetry.sdk.name").map(String::as_str),
        Some("agentops")
    );
}

#[tokio::test]
async fn the_agentops_identity_variables_reach_the_resource() {
    let inputs = catalog_inputs(
        CALLBACKS,
        &[
            API_KEY,
            ("AGENTOPS_SERVICE_NAME", "my-service"),
            ("AGENTOPS_ENVIRONMENT", "staging"),
        ],
    );
    let callback = connect_agentops(&inputs, &FakeTransport::token()).await;
    let shared = SharedOtel::new(&inputs);
    let otel = legacy_otel(&inputs, &shared, &callback).expect("the provider builds");
    let resource = attributes(&otel.resource);
    assert_eq!(
        resource.get("service.name").map(String::as_str),
        Some("my-service")
    );
    assert_eq!(
        resource.get("deployment.environment").map(String::as_str),
        Some("staging")
    );
}

// the operator's OTEL_RESOURCE_ATTRIBUTES survive the composed value and win a name both set
#[tokio::test]
async fn the_operator_resource_attributes_stay_on_the_resource() {
    let inputs = catalog_inputs(
        CALLBACKS,
        &[
            API_KEY,
            (
                "OTEL_RESOURCE_ATTRIBUTES",
                "team=core,service.name=from-operator",
            ),
        ],
    );
    let callback = connect_agentops(&inputs, &FakeTransport::token()).await;
    let shared = SharedOtel::new(&inputs);
    let otel = legacy_otel(&inputs, &shared, &callback).expect("the provider builds");
    let resource = attributes(&otel.resource);
    assert_eq!(resource.get("team").map(String::as_str), Some("core"));
    assert_eq!(
        resource.get("service.name").map(String::as_str),
        Some("from-operator")
    );
    assert_eq!(
        resource.get("project.id").map(String::as_str),
        Some("project-1")
    );
}

// a comma in an AgentOps value would split the attribute list, so the value is percent-encoded
#[tokio::test]
async fn a_comma_in_an_agentops_value_survives_the_resource() {
    let inputs = catalog_inputs(
        CALLBACKS,
        &[API_KEY, ("AGENTOPS_SERVICE_NAME", "my,service")],
    );
    let callback = connect_agentops(&inputs, &FakeTransport::token()).await;
    let shared = SharedOtel::new(&inputs);
    let otel = legacy_otel(&inputs, &shared, &callback).expect("the provider builds");
    assert_eq!(
        attributes(&otel.resource)
            .get("service.name")
            .map(String::as_str),
        Some("my,service")
    );
}

// LiteLLM catches every failure of the fetch and still builds the logger (agentops.py 59-65)
#[tokio::test]
async fn a_refused_token_reply_builds_the_logger_without_a_header() {
    let inputs = catalog_inputs(CALLBACKS, &[API_KEY]);
    let transport = FakeTransport::answering(500, r#"{"error": "boom"}"#);
    let callback = connect_agentops(&inputs, &transport).await;
    assert!(callback.auth.is_none());
    let shared = SharedOtel::new(&inputs);
    let otel = legacy_otel(&inputs, &shared, &callback).expect("the provider builds");
    assert!(otel.config.headers.is_none());
    assert!(!attributes(&otel.resource).contains_key("project.id"));
}

#[tokio::test]
async fn a_tokenless_reply_keeps_the_project_id_without_a_header() {
    let inputs = catalog_inputs(CALLBACKS, &[API_KEY]);
    let transport = FakeTransport::answering(200, PROJECT_ONLY_BODY);
    let callback = connect_agentops(&inputs, &transport).await;
    assert_eq!(transport.requests().len(), 1);
    let shared = SharedOtel::new(&inputs);
    let otel = legacy_otel(&inputs, &shared, &callback).expect("the provider builds");
    assert!(otel.config.headers.is_none());
    assert_eq!(
        attributes(&otel.resource)
            .get("project.id")
            .map(String::as_str),
        Some("project-1")
    );
}

// V2 answers first, so the startup step never reaches the auth endpoint (litellm_logging.py 4514-4516)
#[tokio::test]
async fn v2_with_a_runtime_returns_the_v2_logger() {
    let fixture = v2_runtime();
    let inputs = catalog_inputs(CALLBACKS, &[API_KEY, V2_ON]);
    let transport = FakeTransport::token();
    let callback = connect_agentops(&inputs, &transport).await;
    assert!(
        transport.requests().is_empty(),
        "V2 answers, so no JWT is fetched"
    );
    let catalog = LoggerCatalog::new(inputs)
        .with_runtime(Arc::new(fixture.runtime))
        .with_agentops(callback);
    let logger = catalog
        .resolve_logger(CALLBACK_NAME)
        .expect("the name is available")
        .expect("the V2 logger builds");
    assert_eq!(logger.callback_name(), CALLBACK_NAME);
    assert_eq!(catalog.in_memory_loggers().len(), 1);
    let records = fixture.constructor.records();
    let record = records.first().expect("one V2 build");
    assert_eq!(record.callback_name.as_deref(), Some(CALLBACK_NAME));
}

#[tokio::test]
async fn v2_without_a_runtime_answers_unavailable() {
    let inputs = catalog_inputs(CALLBACKS, &[API_KEY, V2_ON]);
    let transport = FakeTransport::token();
    let callback = connect_agentops(&inputs, &transport).await;
    assert!(transport.requests().is_empty());
    let catalog = LoggerCatalog::new(inputs).with_agentops(callback);
    assert!(matches!(
        catalog.resolve_logger(CALLBACK_NAME),
        Err(RegistryError::UnavailableAdapter { name }) if name == CALLBACK_NAME
    ));
    assert!(catalog.in_memory_loggers().is_empty());
}

// an unreadable switch fails the arm before the legacy path, so the fetch would be wasted
#[tokio::test]
async fn an_unreadable_v2_switch_sends_no_token_request() {
    let inputs = catalog_inputs(CALLBACKS, &[API_KEY, V2_UNREADABLE]);
    let transport = FakeTransport::token();
    let callback = connect_agentops(&inputs, &transport).await;
    assert!(transport.requests().is_empty());
    assert!(callback.auth.is_none());
}

#[test]
fn an_unreadable_v2_switch_fails_the_logger_init() {
    let catalog = catalog(CALLBACKS, &[API_KEY, V2_UNREADABLE]);
    assert!(
        catalog
            .resolve_logger(CALLBACK_NAME)
            .expect("the name is available")
            .is_none()
    );
    assert!(catalog.in_memory_loggers().is_empty());
}

// LiteLLM returns the AgentOps already in _in_memory_loggers before it builds another one
#[test]
fn a_logger_the_catalog_already_built_is_reused() {
    let inputs = catalog_inputs(CALLBACKS, &[API_KEY]);
    let shared = SharedOtel::new(&inputs);
    let connection = idle_callback();
    let existing: Arc<dyn CallbackLogger> = Arc::new(NamedLogger {
        name: CALLBACK_NAME.to_owned(),
    });
    let mut in_memory = vec![Arc::clone(&existing)];
    let built = build(&inputs, None, &shared, &mut in_memory, Some(&connection))
        .expect("the existing logger is reused");
    assert!(Arc::ptr_eq(&existing, &built));
}

#[test]
fn another_arms_logger_is_not_reused() {
    let inputs = catalog_inputs(CALLBACKS, &[API_KEY]);
    let shared = SharedOtel::new(&inputs);
    let connection = idle_callback();
    let other: Arc<dyn CallbackLogger> = Arc::new(NamedLogger {
        name: "opentelemetry".to_owned(),
    });
    let mut in_memory = vec![Arc::clone(&other)];
    let built = build(&inputs, None, &shared, &mut in_memory, Some(&connection))
        .expect("the legacy logger builds");
    assert_eq!(built.callback_name(), CALLBACK_NAME);
    assert!(!Arc::ptr_eq(&other, &built));
}

// the wrapper only intercepts agentops; every other name reaches the catalog's own build
#[test]
fn another_arm_builds_through_the_connected_catalog() {
    let catalog =
        catalog(UNCONFIGURED, &[("OTEL_EXPORTER", "in_memory")]).with_agentops(idle_callback());
    let logger = catalog
        .resolve_logger("otel")
        .expect("the name is available")
        .expect("the legacy logger builds");
    assert_eq!(logger.callback_name(), "opentelemetry");
    assert!(catalog.existing(CALLBACK_NAME).is_none());
}
