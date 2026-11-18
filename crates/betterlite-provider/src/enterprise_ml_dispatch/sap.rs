use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::arguments::json::Json;
use chrono::{DateTime, Utc};
use http::Method;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use url::Url;

use crate::data_cloud_dispatch::context::EnvironmentSource;
use crate::sap_auth::credentials::{Credentials, fetch_credentials, validate_credentials};
use crate::sap_auth::deployment::{
    DeploymentConfigurations, DeploymentMode, orchestration_headers, pick_orchestration_deployment,
};
use crate::sap_auth::environment::SapEnvironment;
use crate::sap_auth::error::{ConfigurationLookupError, SapAuthError};
use crate::sap_auth::token::{
    ClientIdentity, SapClock, SapTokenTransport, TokenCache, TokenRequest, TokenTransportError,
};
use crate::sap_chat::environment::{SapChatCall, SapChatConfig};
use crate::sap_chat::error::SapChatError;
use crate::sap_embedding::environment::{SapEmbeddingCall, SapEmbeddingConfig};
use crate::sap_embedding::error::SapEmbeddingError;

use super::error::EnterpriseError;
use super::transport::EnterpriseTransport;

// LiteLLM's _request_token passes timeout=30
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

// the values the SAP config reads from the call and the litellm_settings section
#[derive(Clone, Copy)]
pub struct SapRequest<'request> {
    pub api_key: Option<&'request SecretString>,
    pub sap_service_key: Option<&'request SecretString>,
    pub env: EnvironmentSource<'request>,
}

#[derive(Debug, Error)]
pub enum SapLookupError {
    #[error(transparent)]
    Auth(#[from] SapAuthError),
    #[error("the {what} request could not be built: {source}")]
    Url {
        what: &'static str,
        #[source]
        source: url::ParseError,
    },
    #[error("the {what} request failed: {source}")]
    Transport {
        what: &'static str,
        #[source]
        source: TransportError,
    },
    #[error("the {what} reply could not be read: {source}")]
    Decode {
        what: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

// LiteLLM's os.environ and Path reads, over the environment snapshot the call runs with
pub struct ProcessEnvironment<'env> {
    pub environment: &'env BTreeMap<String, String>,
}

impl SapEnvironment for ProcessEnvironment<'_> {
    fn var(&self, name: &str) -> Option<String> {
        self.environment
            .get(name)
            .filter(|value| !value.is_empty())
            .cloned()
    }

    fn file_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn read_file(&self, path: &Path) -> Result<String, std::io::Error> {
        std::fs::read_to_string(path)
    }

    fn home_dir(&self) -> PathBuf {
        self.var("HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("/"))
    }
}

pub struct SystemClock;

impl SapClock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

// LiteLLM's _request_token posts the form through _get_httpx_client; the pooled client takes its place
pub struct TokenTransport<'deps> {
    pub transport: &'deps dyn EnterpriseTransport,
}

impl SapTokenTransport for TokenTransport<'_> {
    fn post(&self, request: &TokenRequest) -> Result<String, TokenTransportError> {
        let ClientIdentity::None = request.identity else {
            // the pooled reqwest client cannot present a client certificate, which LiteLLM does through httpx cert=
            return Err(TokenTransportError::Message(String::from(
                "client certificate authentication is not supported by the pooled SAP client",
            )));
        };
        let url = Url::parse(&request.url)
            .map_err(|source| TokenTransportError::Message(source.to_string()))?;
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("grant_type", request.form.grant_type);
        form.append_pair("client_id", &request.form.client_id);
        if let Some(secret) = &request.form.client_secret {
            form.append_pair("client_secret", secret.expose_secret());
        }
        let outbound = OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Json(form.finish().into_bytes()),
            deadline: REQUEST_TIMEOUT,
        };
        let response = await_blocking(self.transport.send(outbound))
            .map_err(|source| TokenTransportError::Transport(Box::new(source)))?;
        Ok(String::from_utf8_lossy(&response.body).into_owned())
    }
}

// LiteLLM's deployment_url property: GET {base_url}/lm/deployments, then one configurations GET per candidate
pub struct Deployments<'deps> {
    pub transport: &'deps dyn EnterpriseTransport,
    pub base_url: String,
    pub headers: HeaderMap,
}

impl DeploymentConfigurations for Deployments<'_> {
    fn configuration(&self, configuration_id: &str) -> Result<Json, ConfigurationLookupError> {
        let missing = || ConfigurationLookupError::Missing(configuration_id.to_owned());
        let url = Url::parse(&format!(
            "{}/lm/configurations/{configuration_id}",
            self.base_url.trim_end_matches('/')
        ))
        .map_err(|_source| missing())?;
        let response = await_blocking(self.transport.send(OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers: self.headers.clone(),
            body: RequestBody::Empty,
            deadline: REQUEST_TIMEOUT,
        }))
        .map_err(|source| ConfigurationLookupError::Transport(Box::new(source)))?;
        serde_json::from_slice::<Json>(&response.body).map_err(ConfigurationLookupError::Decode)
    }
}

// LiteLLM's get_token_creator: fetch_credentials picks the service key, then validate_credentials checks it
fn credentials(
    request: SapRequest<'_>,
    environment: &dyn SapEnvironment,
) -> Result<Credentials, SapAuthError> {
    let service_key = request
        .api_key
        .filter(|key| !key.expose_secret().is_empty())
        .map(|key| Json::Text(key.expose_secret().to_owned()));
    let litellm_key = request
        .sap_service_key
        .map(|key| Json::Text(key.expose_secret().to_owned()));
    let resolved = fetch_credentials(environment, service_key.as_ref(), litellm_key.as_ref())?;
    validate_credentials(&resolved)?;
    Ok(resolved)
}

fn deployment_url(
    transport: &dyn EnterpriseTransport,
    credentials: &Credentials,
    headers: &HeaderMap,
    mode: DeploymentMode,
) -> Result<String, SapLookupError> {
    let base = credentials
        .base_url
        .as_deref()
        .filter(|url| !url.is_empty())
        .ok_or(SapAuthError::MissingCredentials)?;
    let url = Url::parse(&format!("{}/lm/deployments", base.trim_end_matches('/'))).map_err(
        |source| SapLookupError::Url {
            what: "deployments",
            source,
        },
    )?;
    let response = await_blocking(transport.send(OutboundRequest {
        method: Method::GET,
        url,
        query: Vec::new(),
        headers: headers.clone(),
        body: RequestBody::Empty,
        deadline: REQUEST_TIMEOUT,
    }))
    .map_err(|source| SapLookupError::Transport {
        what: "deployments",
        source,
    })?;
    let listing = serde_json::from_slice::<Json>(&response.body).map_err(|source| {
        SapLookupError::Decode {
            what: "deployments",
            source,
        }
    })?;
    let configurations = Deployments {
        transport,
        base_url: base.to_owned(),
        headers: headers.clone(),
    };
    Ok(pick_orchestration_deployment(
        &listing,
        mode,
        &configurations,
    )?)
}

fn session(
    request: SapRequest<'_>,
    transport: &dyn EnterpriseTransport,
    clock: &dyn SapClock,
    mode: DeploymentMode,
) -> Result<(SecretString, String, String), EnterpriseError> {
    let environment = ProcessEnvironment {
        environment: request.env.environment,
    };
    let credentials = credentials(request, &environment)?;
    let token = TokenCache::new(&credentials)?.token(clock, &TokenTransport { transport })?;
    let headers = orchestration_headers(&token, &credentials.resource_group)?;
    let deployment = deployment_url(transport, &credentials, &headers, mode)?;
    Ok((token, credentials.resource_group, deployment))
}

pub fn prepare_chat(
    request: SapRequest<'_>,
    transport: &dyn EnterpriseTransport,
    clock: &dyn SapClock,
) -> Result<SapChatCall, EnterpriseError> {
    match session(request, transport, clock, DeploymentMode::Chat) {
        Ok((token, resource_group, deployment_url)) => Ok(SapChatConfig {
            token,
            resource_group,
            deployment_url,
        }
        .prepare()?),
        // LiteLLM wraps the credentials ValueError as a 400 and lets every other setup failure through
        Err(EnterpriseError::SapAuth(error)) => Err(SapChatError::credentials(error).into()),
        Err(error) => Err(error),
    }
}

pub fn prepare_embedding(
    request: SapRequest<'_>,
    transport: &dyn EnterpriseTransport,
    clock: &dyn SapClock,
) -> Result<SapEmbeddingCall, EnterpriseError> {
    match session(request, transport, clock, DeploymentMode::Embedding) {
        Ok((token, resource_group, deployment_url)) => Ok(SapEmbeddingConfig {
            token,
            resource_group,
            deployment_url,
        }
        .prepare()?),
        Err(EnterpriseError::SapAuth(error)) => Err(SapEmbeddingError::Auth(error).into()),
        Err(error) => Err(error),
    }
}

// the SAP parts are sync, as B5950 declares them; the pooled client is async, so the call blocks on it here
pub fn await_blocking<F>(future: F) -> F::Output
where
    F: Future,
{
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
}
