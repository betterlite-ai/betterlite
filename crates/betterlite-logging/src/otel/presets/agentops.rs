use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use http::header::{AUTHORIZATION, CONNECTION, CONTENT_TYPE, HeaderValue};
use http::{Method, StatusCode};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig as _, WithHttpConfig as _};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;
use tokio::time::timeout;

use crate::otel::config::{ExporterOwner, ExporterSpec, Settings};
use crate::otel::plumbing::exporters::{ExportResult, ExporterFactory, HttpClients};
use crate::otel::plumbing::sinks::SpanSink;
use crate::otel::presets::base::{Preset, PresetError};

pub const AGENTOPS_EXPORTER_KIND: &str = "agentops";

const AGENTOPS_ENDPOINT: &str = "https://otlp.agentops.ai/v1/traces";
const AGENTOPS_AUTH_ENDPOINT: &str = "https://api.agentops.ai/v3/auth/token";
const API_KEY_OPTION: &str = "api_key";
const FAILURE_PREFIX: &str = "AgentOps JWT fetch failed";
const TOKEN_TIMEOUT: Duration = Duration::from_secs(10);

pub struct AgentopsPreset;

impl Preset for AgentopsPreset {
    fn build(
        &self,
        environment: &dyn EnvironmentLookup,
        base: Settings,
        _allow_missing_credentials: bool,
    ) -> Result<Settings, PresetError> {
        // An empty service name is kept: only an unset variable takes the default
        let api_key = environment
            .lookup("AGENTOPS_API_KEY")
            .filter(|value| !value.is_empty());
        let service_name = environment
            .lookup("AGENTOPS_SERVICE_NAME")
            .unwrap_or_else(|| "agentops".to_owned());
        let deployment_environment = environment
            .lookup("AGENTOPS_ENVIRONMENT")
            .filter(|value| !value.is_empty());
        let mut settings = base;
        settings.exporters.push(ExporterSpec {
            kind: AGENTOPS_EXPORTER_KIND.to_owned(),
            endpoint: Some(AGENTOPS_ENDPOINT.to_owned()),
            traces_endpoint: None,
            headers: None,
            owner: Some(ExporterOwner::Agentops),
            options: api_key.map(|key| BTreeMap::from([(API_KEY_OPTION.to_owned(), key)])),
            use_simple_processor: None,
            requires_headers: false,
        });
        // The JWT carries the project, so no project id resource attribute is added
        settings
            .resource_attributes
            .insert("service.name".to_owned(), service_name);
        settings
            .resource_attributes
            .insert("telemetry.sdk.name".to_owned(), "agentops".to_owned());
        if let Some(name) = deployment_environment {
            settings
                .resource_attributes
                .insert("deployment.environment".to_owned(), name);
        }
        Ok(settings)
    }
}

#[must_use]
pub fn agentops_exporter_factory(clients: &HttpClients) -> ExporterFactory {
    Arc::new({
        let client = Arc::clone(&clients.traces);
        move |spec: &ExporterSpec| build_exporter(&client, spec)
    })
}

fn build_exporter(client: &Arc<dyn HttpClient>, spec: &ExporterSpec) -> ExportResult<SpanSink> {
    let api_key = spec
        .options
        .as_ref()
        .and_then(|options| options.get(API_KEY_OPTION))
        .filter(|key| !key.is_empty())
        .map(|key| SecretString::from(key.clone()));
    let auth = AgentopsAuthClient {
        inner: Arc::clone(client),
        api_key,
        token: OnceCell::new(),
    };
    let mut builder = SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .with_shared_http_client(Arc::new(auth));
    // The spec's endpoint is used exactly as given; only an env/default endpoint gains /v1/traces
    if let Some(endpoint) = spec
        .endpoint
        .as_deref()
        .filter(|endpoint| !endpoint.is_empty())
    {
        builder = builder.with_endpoint(endpoint);
    }
    Ok(SpanSink::Otlp(Box::new(builder.build()?)))
}

#[derive(Debug)]
struct AgentopsAuthClient {
    inner: Arc<dyn HttpClient>,
    api_key: Option<SecretString>,
    token: OnceCell<Option<SecretString>>,
}

#[async_trait]
impl HttpClient for AgentopsAuthClient {
    async fn send_bytes(&self, mut request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let cached = self.token.get_or_init(|| self.fetch_token()).await;
        if let Some(token) = cached
            && let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", token.expose_secret()))
        {
            request.headers_mut().insert(AUTHORIZATION, value);
        }
        self.inner.send_bytes(request).await
    }
}

impl AgentopsAuthClient {
    // One attempt per exporter: the cell keeps the failure too, so a dead token endpoint is not retried per batch
    async fn fetch_token(&self) -> Option<SecretString> {
        let Some(api_key) = &self.api_key else {
            tracing::debug!("{FAILURE_PREFIX}: no api key was configured");
            return None;
        };
        let request = token_request(api_key.expose_secret())?;
        match timeout(TOKEN_TIMEOUT, self.inner.send_bytes(request)).await {
            Ok(Ok(response)) => token_from_response(&response),
            Ok(Err(error)) => {
                tracing::debug!("{FAILURE_PREFIX}: the request failed: {error}");
                None
            }
            Err(_elapsed) => {
                tracing::debug!("{FAILURE_PREFIX}: the request timed out");
                None
            }
        }
    }
}

#[derive(Serialize)]
struct TokenRequest<'key> {
    api_key: &'key str,
}

#[derive(Deserialize)]
struct TokenResponse {
    token: Option<String>,
}

// Every failure is a debug line and no header, as LiteLLM catches every exception of the fetch
fn token_request(api_key: &str) -> Option<Request<Bytes>> {
    let body = match serde_json::to_vec(&TokenRequest { api_key }) {
        Ok(body) => body,
        Err(error) => {
            tracing::debug!("{FAILURE_PREFIX}: the body could not be encoded: {error}");
            return None;
        }
    };
    let built = Request::builder()
        .method(Method::POST)
        .uri(AGENTOPS_AUTH_ENDPOINT)
        .header(CONTENT_TYPE, "application/json")
        .header(CONNECTION, "keep-alive")
        .body(Bytes::from(body));
    match built {
        Ok(request) => Some(request),
        Err(error) => {
            tracing::debug!("{FAILURE_PREFIX}: the request could not be built: {error}");
            None
        }
    }
}

fn token_from_response(response: &Response<Bytes>) -> Option<SecretString> {
    if response.status() != StatusCode::OK {
        tracing::debug!(
            "{FAILURE_PREFIX}: the token endpoint answered with status {}",
            response.status()
        );
        return None;
    }
    let payload: TokenResponse = match serde_json::from_slice(response.body()) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::debug!("{FAILURE_PREFIX}: the response is not valid JSON: {error}");
            return None;
        }
    };
    let Some(token) = payload.token.filter(|token| !token.is_empty()) else {
        tracing::debug!("{FAILURE_PREFIX}: the response has no non-empty token");
        return None;
    };
    if let Err(error) = HeaderValue::from_str(&format!("Bearer {token}")) {
        tracing::debug!("{FAILURE_PREFIX}: the token is not a valid header value: {error}");
        return None;
    }
    Some(SecretString::from(token))
}

mod tests;
