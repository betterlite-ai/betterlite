use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use http::header::{CONNECTION, CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use crate::otel::legacy_policy::config::{
    Exporter, OTLP_HTTP_EXPORTER, OpenTelemetryConfig, Signals,
};

pub mod transport;

mod tests;

use self::transport::{AuthPost, AuthTransport};

pub const ENDPOINT: &str = "https://otlp.agentops.cloud/v1/traces";
pub const AUTH_ENDPOINT: &str = "https://api.agentops.ai/v3/auth/token";
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_SERVICE_NAME: &str = "agentops";
pub const DEFAULT_DEPLOYMENT_ENVIRONMENT: &str = "production";
pub const RESOURCE_SERVICE_NAME: &str = "litellm";
pub const RESOURCE_ENVIRONMENT: &str = "production";
pub const SDK_NAME: &str = "agentops";

#[derive(Clone, Debug)]
pub struct AgentOpsConfig {
    pub endpoint: String,
    pub api_key: Option<SecretString>,
    pub service_name: Option<String>,
    pub deployment_environment: Option<String>,
    pub auth_endpoint: String,
}

impl Default for AgentOpsConfig {
    fn default() -> Self {
        Self {
            endpoint: ENDPOINT.to_owned(),
            api_key: None,
            service_name: None,
            deployment_environment: None,
            auth_endpoint: AUTH_ENDPOINT.to_owned(),
        }
    }
}

impl AgentOpsConfig {
    #[must_use]
    pub fn from_env(environment: &dyn EnvironmentLookup) -> Self {
        let read = |name: &str| environment.lookup(name).filter(|value| !value.is_empty());
        Self {
            endpoint: ENDPOINT.to_owned(),
            api_key: read("AGENTOPS_API_KEY").map(SecretString::from),
            service_name: Some(
                read("AGENTOPS_SERVICE_NAME").unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_owned()),
            ),
            deployment_environment: Some(
                read("AGENTOPS_ENVIRONMENT")
                    .unwrap_or_else(|| DEFAULT_DEPLOYMENT_ENVIRONMENT.to_owned()),
            ),
            auth_endpoint: AUTH_ENDPOINT.to_owned(),
        }
    }
}

// the two values LiteLLM reads out of the token response; either may be missing on its own
#[derive(Clone, Debug, Default)]
pub struct AgentOpsAuth {
    pub token: Option<SecretString>,
    pub project_id: Option<String>,
}

pub struct AgentOpsCallback {
    pub config: AgentOpsConfig,
    pub auth: Option<AgentOpsAuth>,
}

impl AgentOpsCallback {
    // LiteLLM catches every failure of the token fetch and keeps the callback with no auth at all
    pub async fn connect(config: AgentOpsConfig, transport: &dyn AuthTransport) -> Self {
        let api_key = config
            .api_key
            .as_ref()
            .filter(|key| !key.expose_secret().is_empty());
        let auth = match api_key {
            Some(key) => fetch_auth(transport, key, &config.auth_endpoint).await,
            None => None,
        };
        Self { config, auth }
    }

    #[must_use]
    pub fn resource_attributes(&self) -> BTreeMap<String, String> {
        let mut attributes = BTreeMap::from([
            (
                "service.name".to_owned(),
                self.config
                    .service_name
                    .as_deref()
                    .filter(|text| !text.is_empty())
                    .unwrap_or(RESOURCE_SERVICE_NAME)
                    .to_owned(),
            ),
            (
                "deployment.environment".to_owned(),
                self.config
                    .deployment_environment
                    .as_deref()
                    .filter(|text| !text.is_empty())
                    .unwrap_or(RESOURCE_ENVIRONMENT)
                    .to_owned(),
            ),
            ("telemetry.sdk.name".to_owned(), SDK_NAME.to_owned()),
        ]);
        let project_id = self
            .auth
            .as_ref()
            .and_then(|auth| auth.project_id.as_deref())
            .filter(|project| !project.is_empty());
        if let Some(project_id) = project_id {
            attributes.insert("project.id".to_owned(), project_id.to_owned());
        }
        attributes
    }

    #[must_use]
    pub fn otel_config(&self, env: &BTreeMap<String, String>) -> OpenTelemetryConfig {
        let mut config = OpenTelemetryConfig::from_env(env);
        config.exporter = Exporter::Named(OTLP_HTTP_EXPORTER.to_owned());
        config.endpoint = Some(self.config.endpoint.clone());
        config.headers = self
            .auth
            .as_ref()
            .and_then(|auth| auth.token.as_ref())
            .map(|token| format!("Authorization=Bearer {}", token.expose_secret()));
        // the Python dataclass leaves both signal switches false whatever the environment says
        config.signals = Signals {
            enable_metrics: false,
            enable_events: false,
        };
        // __post_init__ fills the remaining identity fields from the OTEL_* variables
        config.normalize(env);
        config
    }
}

#[derive(Serialize)]
struct TokenRequest<'key> {
    api_key: &'key str,
}

#[derive(Deserialize)]
struct TokenResponse {
    token: Option<String>,
    project_id: Option<String>,
}

// every failure means no Authorization header and no project id, exactly as LiteLLM's bare except leaves it
async fn fetch_auth(
    transport: &dyn AuthTransport,
    api_key: &SecretString,
    auth_endpoint: &str,
) -> Option<AgentOpsAuth> {
    let body = serde_json::to_vec(&TokenRequest {
        api_key: api_key.expose_secret(),
    })
    .ok()?;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(CONNECTION, HeaderValue::from_static("keep-alive"));
    let request = AuthPost {
        url: auth_endpoint.to_owned(),
        headers,
        body,
        timeout: AUTH_TIMEOUT,
    };
    let response = match transport.post(request).await {
        Ok(response) => response,
        Err(error) => {
            tracing::debug!("AgentOps JWT fetch failed: {error}");
            return None;
        }
    };
    if response.status != 200 {
        tracing::debug!(
            "AgentOps JWT fetch was answered with status {}",
            response.status
        );
        return None;
    }
    let payload: TokenResponse = serde_json::from_slice(&response.body).ok()?;
    Some(AgentOpsAuth {
        token: payload
            .token
            .filter(|token| !token.is_empty())
            .map(SecretString::from),
        project_id: payload.project_id.filter(|project| !project.is_empty()),
    })
}
