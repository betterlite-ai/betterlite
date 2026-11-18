#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use http::Method;
use http::header::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

use super::transport::{AuthFailure, AuthPost, AuthResponse, AuthTransport, PooledAuthTransport};
use super::{
    AUTH_ENDPOINT, AUTH_TIMEOUT, AgentOpsCallback, AgentOpsConfig, DEFAULT_DEPLOYMENT_ENVIRONMENT,
    DEFAULT_SERVICE_NAME, ENDPOINT, RESOURCE_ENVIRONMENT, RESOURCE_SERVICE_NAME, SDK_NAME,
};
use crate::otel::callbacks::tests::json;
use crate::otel::legacy_policy::config::{Exporter, OTLP_HTTP_EXPORTER};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn config(auth_endpoint: &str, api_key: Option<&str>) -> AgentOpsConfig {
    AgentOpsConfig {
        endpoint: ENDPOINT.to_owned(),
        api_key: api_key.map(SecretString::from),
        service_name: None,
        deployment_environment: None,
        auth_endpoint: auth_endpoint.to_owned(),
    }
}

fn client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    ProxiedClient::outbound(&policy).expect("http client")
}

fn exposed(token: Option<&SecretString>) -> Option<&str> {
    token.map(|token| token.expose_secret())
}

struct Received {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

// the future accepts one request and answers it; the test joins it with the call under test
async fn serve_once(status: u16, body: &'static str) -> (String, impl Future<Output = Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let address = listener.local_addr().expect("the bound address");
    let response = format!(
        "HTTP/1.1 {status} AgentOps\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let served = async move {
        let (mut stream, _) = listener.accept().await.expect("one request");
        let received = receive(&mut stream).await;
        stream
            .write_all(response.as_bytes())
            .await
            .expect("the response write");
        stream.flush().await.expect("the response flush");
        received
    };
    (format!("http://{address}/v3/auth/token"), served)
}

async fn receive(stream: &mut TcpStream) -> Received {
    let mut buffer = Vec::new();
    let head_end = loop {
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        read_more(stream, &mut buffer).await;
    };
    let head = String::from_utf8(buffer[..head_end].to_vec()).expect("a UTF-8 request head");
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().expect("a request line").split(' ');
    let method = request_line.next().expect("a method").to_owned();
    let path = request_line.next().expect("a path").to_owned();
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
        .collect();
    let length = headers
        .get("content-length")
        .map_or(0, |value| value.parse::<usize>().expect("a content length"));
    let mut body = buffer[head_end..].to_vec();
    while body.len() < length {
        read_more(stream, &mut body).await;
    }
    Received {
        method,
        path,
        headers,
        body,
    }
}

async fn read_more(stream: &mut TcpStream, buffer: &mut Vec<u8>) {
    let mut chunk = [0_u8; 2048];
    let read = stream.read(&mut chunk).await.expect("a request read");
    assert!(read > 0, "the client closed before sending a request");
    buffer.extend_from_slice(&chunk[..read]);
}

fn header<'headers>(received: &'headers Received, name: &str) -> &'headers str {
    received
        .headers
        .get(name)
        .map_or_else(|| panic!("the request carries {name}"), String::as_str)
}

struct TokenTransport;

#[async_trait]
impl AuthTransport for TokenTransport {
    async fn post(&self, _request: AuthPost) -> Result<AuthResponse, AuthFailure> {
        Ok(AuthResponse {
            status: 200,
            body: br#"{"token": "unexpected-token", "project_id": "unexpected-project"}"#.to_vec(),
        })
    }
}

#[derive(Default)]
struct CaptureTransport {
    requests: Mutex<Vec<AuthPost>>,
}

#[async_trait]
impl AuthTransport for CaptureTransport {
    async fn post(&self, request: AuthPost) -> Result<AuthResponse, AuthFailure> {
        self.requests
            .lock()
            .expect("the captured requests lock")
            .push(request);
        Ok(AuthResponse {
            status: 200,
            body: br#"{"token": "jwt-1", "project_id": "project-1"}"#.to_vec(),
        })
    }
}

#[tokio::test]
async fn agentops_callback_fetches_the_token_with_the_documented_request_shape() {
    let transport = CaptureTransport::default();
    let callback =
        AgentOpsCallback::connect(config(AUTH_ENDPOINT, Some("agentops-key")), &transport).await;
    let requests = transport
        .requests
        .lock()
        .expect("the captured requests lock");
    let request = requests.first().expect("one token request");
    assert_eq!(request.url, AUTH_ENDPOINT);
    assert_eq!(
        request
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(
        request
            .headers
            .get("connection")
            .and_then(|value| value.to_str().ok()),
        Some("keep-alive")
    );
    assert_eq!(request.timeout, AUTH_TIMEOUT);
    assert_eq!(
        serde_json::from_slice::<Json>(&request.body).expect("the body is JSON"),
        json(r#"{"api_key": "agentops-key"}"#)
    );
    assert!(callback.auth.is_some());
}

#[tokio::test]
async fn agentops_callback_posts_the_api_key_and_keeps_the_token() {
    let (endpoint, served) =
        serve_once(200, r#"{"token": "jwt-1", "project_id": "project-1"}"#).await;
    let transport = PooledAuthTransport { client: client() };
    let (callback, received) = tokio::join!(
        AgentOpsCallback::connect(config(&endpoint, Some("agentops-key")), &transport),
        served
    );
    assert_eq!(received.method, "POST");
    assert_eq!(received.path, "/v3/auth/token");
    assert_eq!(header(&received, "content-type"), "application/json");
    assert_eq!(
        serde_json::from_slice::<Json>(&received.body).expect("the body is JSON"),
        json(r#"{"api_key": "agentops-key"}"#)
    );
    let auth = callback.auth.expect("the token response is kept");
    assert_eq!(exposed(auth.token.as_ref()), Some("jwt-1"));
    assert_eq!(auth.project_id.as_deref(), Some("project-1"));
}

#[tokio::test]
async fn agentops_callback_skips_the_token_request_without_an_api_key() {
    let transport = TokenTransport;
    let callback = AgentOpsCallback::connect(config(AUTH_ENDPOINT, None), &transport).await;
    assert!(callback.auth.is_none());
}

#[tokio::test]
async fn agentops_callback_skips_the_token_request_for_an_empty_api_key() {
    let transport = TokenTransport;
    let callback = AgentOpsCallback::connect(config(AUTH_ENDPOINT, Some("")), &transport).await;
    assert!(callback.auth.is_none());
}

#[test]
fn agentops_callback_reads_the_api_key_and_service_identity_from_the_environment() {
    let config = AgentOpsConfig::from_env(&environment(&[
        ("AGENTOPS_API_KEY", "agentops-key"),
        ("AGENTOPS_SERVICE_NAME", "my-service"),
        ("AGENTOPS_ENVIRONMENT", "staging"),
    ]));
    assert_eq!(config.endpoint, ENDPOINT);
    assert_eq!(config.auth_endpoint, AUTH_ENDPOINT);
    assert_eq!(exposed(config.api_key.as_ref()), Some("agentops-key"));
    assert_eq!(config.service_name.as_deref(), Some("my-service"));
    assert_eq!(config.deployment_environment.as_deref(), Some("staging"));
}

#[test]
fn agentops_callback_defaults_the_service_identity_from_the_environment() {
    let config = AgentOpsConfig::from_env(&environment(&[]));
    assert!(config.api_key.is_none());
    assert_eq!(config.service_name.as_deref(), Some(DEFAULT_SERVICE_NAME));
    assert_eq!(
        config.deployment_environment.as_deref(),
        Some(DEFAULT_DEPLOYMENT_ENVIRONMENT)
    );
}

#[test]
fn agentops_callback_treats_an_empty_environment_value_as_unset() {
    let config = AgentOpsConfig::from_env(&environment(&[
        ("AGENTOPS_API_KEY", ""),
        ("AGENTOPS_SERVICE_NAME", ""),
        ("AGENTOPS_ENVIRONMENT", ""),
    ]));
    assert!(config.api_key.is_none());
    assert_eq!(config.service_name.as_deref(), Some(DEFAULT_SERVICE_NAME));
    assert_eq!(
        config.deployment_environment.as_deref(),
        Some(DEFAULT_DEPLOYMENT_ENVIRONMENT)
    );
}

#[tokio::test]
async fn agentops_callback_falls_back_silently_when_the_token_endpoint_refuses() {
    let (endpoint, served) = serve_once(401, r#"{"error": "unauthorized"}"#).await;
    let transport = PooledAuthTransport { client: client() };
    let (callback, _received) = tokio::join!(
        AgentOpsCallback::connect(config(&endpoint, Some("agentops-key")), &transport),
        served
    );
    assert!(callback.auth.is_none());
    assert!(!callback.resource_attributes().contains_key("project.id"));
}

#[tokio::test]
async fn agentops_callback_falls_back_silently_on_a_malformed_token_response() {
    let (endpoint, served) = serve_once(200, "this is not JSON").await;
    let transport = PooledAuthTransport { client: client() };
    let (callback, _received) = tokio::join!(
        AgentOpsCallback::connect(config(&endpoint, Some("agentops-key")), &transport),
        served
    );
    assert!(callback.auth.is_none());
}

#[tokio::test]
async fn agentops_callback_falls_back_silently_when_the_endpoint_is_unreachable() {
    let transport = PooledAuthTransport { client: client() };
    let callback = AgentOpsCallback::connect(
        config("http://127.0.0.1:1/v3/auth/token", Some("agentops-key")),
        &transport,
    )
    .await;
    assert!(callback.auth.is_none());
}

#[tokio::test]
async fn agentops_callback_falls_back_silently_when_the_endpoint_is_not_a_url() {
    let transport = PooledAuthTransport { client: client() };
    let callback =
        AgentOpsCallback::connect(config("not a url", Some("agentops-key")), &transport).await;
    assert!(callback.auth.is_none());
}

#[tokio::test]
async fn agentops_callback_keeps_the_project_id_without_a_token() {
    let (endpoint, served) = serve_once(200, r#"{"project_id": "project-1"}"#).await;
    let transport = PooledAuthTransport { client: client() };
    let (callback, _received) = tokio::join!(
        AgentOpsCallback::connect(config(&endpoint, Some("agentops-key")), &transport),
        served
    );
    let auth = callback.auth.expect("the project id is kept");
    assert!(auth.token.is_none());
    assert_eq!(auth.project_id.as_deref(), Some("project-1"));
    assert!(callback.otel_config(&environment(&[])).headers.is_none());
    assert_eq!(
        callback
            .resource_attributes()
            .get("project.id")
            .map(String::as_str),
        Some("project-1")
    );
}

#[test]
fn agentops_callback_resource_attributes_default_to_litellm_and_production() {
    let callback = AgentOpsCallback {
        config: AgentOpsConfig::default(),
        auth: None,
    };
    assert_eq!(
        callback.resource_attributes(),
        BTreeMap::from([
            (
                "deployment.environment".to_owned(),
                RESOURCE_ENVIRONMENT.to_owned()
            ),
            ("service.name".to_owned(), RESOURCE_SERVICE_NAME.to_owned()),
            ("telemetry.sdk.name".to_owned(), SDK_NAME.to_owned()),
        ])
    );
}

#[test]
fn agentops_callback_resource_attributes_keep_a_configured_service_identity() {
    let callback = AgentOpsCallback {
        config: AgentOpsConfig {
            endpoint: ENDPOINT.to_owned(),
            api_key: None,
            service_name: Some("my-service".to_owned()),
            deployment_environment: Some("staging".to_owned()),
            auth_endpoint: AUTH_ENDPOINT.to_owned(),
        },
        auth: None,
    };
    let attributes = callback.resource_attributes();
    assert_eq!(
        attributes.get("service.name").map(String::as_str),
        Some("my-service")
    );
    assert_eq!(
        attributes.get("deployment.environment").map(String::as_str),
        Some("staging")
    );
}

#[tokio::test]
async fn agentops_callback_resource_attributes_carry_the_fetched_project_id() {
    let (endpoint, served) =
        serve_once(200, r#"{"token": "jwt-1", "project_id": "project-1"}"#).await;
    let transport = PooledAuthTransport { client: client() };
    let (callback, _received) = tokio::join!(
        AgentOpsCallback::connect(config(&endpoint, Some("agentops-key")), &transport),
        served
    );
    assert_eq!(
        callback
            .resource_attributes()
            .get("project.id")
            .map(String::as_str),
        Some("project-1")
    );
}

#[tokio::test]
async fn agentops_callback_otel_config_points_at_the_traces_endpoint_with_the_bearer_header() {
    let (endpoint, served) = serve_once(200, r#"{"token": "jwt-1"}"#).await;
    let transport = PooledAuthTransport { client: client() };
    let (callback, _received) = tokio::join!(
        AgentOpsCallback::connect(config(&endpoint, Some("agentops-key")), &transport),
        served
    );
    let config = callback.otel_config(&environment(&[("OTEL_SERVICE_NAME", "gateway")]));
    assert_eq!(
        config.exporter,
        Exporter::Named(OTLP_HTTP_EXPORTER.to_owned())
    );
    assert_eq!(config.endpoint.as_deref(), Some(ENDPOINT));
    assert_eq!(
        config.headers.as_deref(),
        Some("Authorization=Bearer jwt-1")
    );
    assert_eq!(config.service_name, "gateway");
}

#[test]
fn agentops_callback_otel_config_leaves_both_signal_switches_off() {
    let callback = AgentOpsCallback {
        config: AgentOpsConfig::default(),
        auth: None,
    };
    let config = callback.otel_config(&environment(&[
        ("LITELLM_OTEL_INTEGRATION_ENABLE_METRICS", "true"),
        ("LITELLM_OTEL_INTEGRATION_ENABLE_EVENTS", "true"),
    ]));
    assert!(!config.signals.enable_metrics);
    assert!(!config.signals.enable_events);
}

#[tokio::test]
async fn agentops_callback_leaves_the_pooled_client_usable_after_the_token_request() {
    let (endpoint, served) = serve_once(200, r#"{"token": "jwt-1"}"#).await;
    let client = client();
    let transport = PooledAuthTransport {
        client: client.clone(),
    };
    let (_callback, _received) = tokio::join!(
        AgentOpsCallback::connect(config(&endpoint, Some("agentops-key")), &transport),
        served
    );
    let (next_endpoint, served_again) = serve_once(200, "{}").await;
    let request = OutboundRequest {
        method: Method::GET,
        url: Url::parse(&next_endpoint).expect("the endpoint parses"),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: Duration::from_secs(10),
    };
    let (response, received) = tokio::join!(client.send_buffered(request), served_again);
    assert_eq!(
        response.expect("the pooled client still answers").status,
        200
    );
    assert_eq!(received.path, "/v3/auth/token");
}
