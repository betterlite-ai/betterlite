use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_core::a2a::exception_mapping::{
    A2aSdk, CardUrls, ClientConfig, ClientStash, map_a2a_exception,
};
use betterlite_core::agents::json::AgentJson;
use betterlite_http::proxy::{ProxiedClient, limits::TransportLimits};
use secrecy::SecretString;
use url::Url;

use crate::agent_card::AgentCard;
use crate::card_resolver::{CardResolver, card_url, interfaces::normalize_interfaces, localhost};
use crate::client::error::{ClientError, exception_to_a2a};
use crate::client::interface::selected_interface;
use crate::errors::A2AError;
use crate::errors::retries::RetryCounts;

pub mod convert;
pub mod error;
mod interface;
pub mod records;
pub mod retry;
pub mod send;
mod tests;
pub mod transport;

pub const DEFAULT_A2A_AGENT_TIMEOUT: Duration = Duration::from_mins(100);
pub const A2A_TIMEOUT_ENV: &str = "DEFAULT_A2A_AGENT_TIMEOUT";

#[derive(Clone, Debug, Default)]
pub struct ClientCallContext {
    pub service_parameters: Option<BTreeMap<String, SecretString>>,
}

#[derive(Clone, Debug)]
pub struct A2AClientSettings {
    pub base_url: String,
    pub timeout: Option<Duration>,
    pub extra_headers: BTreeMap<String, SecretString>,
    pub streaming: bool,
    pub relative_card_path: Option<String>,
}

pub struct A2AClient {
    pub http_client: ProxiedClient,
    pub call_context: Option<ClientCallContext>,
    pub agent_card: AgentCard,
    pub streaming: bool,
    pub endpoint: Url,
    pub timeout: Duration,
    pub tenant: Option<String>,
    pub request_ids: Arc<Mutex<Box<dyn RequestIds>>>,
}

pub trait RequestIds: Send {
    fn next_request_id(&mut self) -> String;
}

#[derive(Debug, Default)]
pub struct UuidRequestIds;

impl RequestIds for UuidRequestIds {
    fn next_request_id(&mut self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NativeA2aSdk;

pub async fn create_a2a_client(
    http: &ProxiedClient,
    settings: A2AClientSettings,
    environment: &(dyn EnvironmentLookup + Sync),
) -> Result<A2AClient, A2AError> {
    let timeout = resolve_timeout(environment, settings.timeout)?;
    let effective = effective_client(http, timeout);
    let headers = transport::headers(Some(&settings.extra_headers), false)
        .map_err(|error| A2AError::from(&error))?;
    let resolver = CardResolver {
        base_url: settings.base_url,
        client: effective.clone(),
    };
    let found = resolver
        .get_agent_card(settings.relative_card_path.as_deref(), &headers)
        .await?;
    let card = normalize_interfaces(found);
    let call_context = (!settings.extra_headers.is_empty()).then_some(ClientCallContext {
        service_parameters: Some(settings.extra_headers),
    });
    let stash = ClientStash {
        httpx_client: effective.clone(),
        call_context,
        agent_card: card.clone(),
    };
    let config = ClientConfig {
        httpx_client: effective,
        streaming: settings.streaming,
    };
    let mut client = NativeA2aSdk
        .create_client(&card, config)
        .await
        .map_err(|error| A2AError::from(&error))?;
    NativeA2aSdk.attach_stash(&mut client, stash);
    Ok(client)
}

#[must_use]
pub fn agent_card_path(litellm_params: &BTreeMap<String, AgentJson>) -> Option<String> {
    litellm_params
        .get("agent_card_path")
        .and_then(AgentJson::as_str)
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
}

impl From<&ClientError> for A2AError {
    fn from(error: &ClientError) -> Self {
        exception_to_a2a(map_a2a_exception(&NativeA2aSdk, error, None, None, None))
    }
}

fn effective_client(pool: &ProxiedClient, timeout: Duration) -> ProxiedClient {
    ProxiedClient {
        limits: TransportLimits {
            connect_timeout: timeout,
            read_timeout: timeout,
            ..pool.limits
        },
        ..pool.clone()
    }
}

// LiteLLM's DEFAULT_A2A_AGENT_TIMEOUT is read from the environment when the caller gives no timeout
fn resolve_timeout(
    environment: &(dyn EnvironmentLookup + Sync),
    configured: Option<Duration>,
) -> Result<Duration, A2AError> {
    if let Some(timeout) = configured {
        return Ok(timeout);
    }
    let Some(raw) = environment
        .lookup(A2A_TIMEOUT_ENV)
        .filter(|raw| !raw.is_empty())
    else {
        return Ok(DEFAULT_A2A_AGENT_TIMEOUT);
    };
    let seconds = raw
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0_f64);
    let Some(timeout) = seconds.and_then(|value| Duration::try_from_secs_f64(value).ok()) else {
        return Err(A2AError::Base {
            message: format!("{A2A_TIMEOUT_ENV} {raw:?} is not a valid number of seconds"),
            model: None,
            debug_info: None,
            retries: RetryCounts::default(),
        });
    };
    Ok(timeout)
}

impl CardUrls for NativeA2aSdk {
    type Card = AgentCard;

    fn is_localhost_or_internal_url(&self, url: Option<&str>) -> bool {
        localhost::is_localhost_or_internal_url(url)
    }

    fn set_agent_card_url(&self, card: &mut AgentCard, url: &str) {
        card_url::set_agent_card_url(card, url);
    }
}

impl A2aSdk for NativeA2aSdk {
    const A2A_SDK_AVAILABLE: bool = true;

    type Client = A2AClient;
    type HttpClient = ProxiedClient;
    type CallContext = ClientCallContext;
    type SdkError = ClientError;

    fn httpx_client(&self, client: &A2AClient) -> Option<ProxiedClient> {
        Some(client.http_client.clone())
    }

    fn call_context(&self, client: &A2AClient) -> Option<ClientCallContext> {
        client.call_context.clone()
    }

    fn create_client(
        &self,
        card: &AgentCard,
        config: ClientConfig<ProxiedClient>,
    ) -> impl Future<Output = Result<A2AClient, ClientError>> + Send {
        let endpoint = selected_interface(card)
            .ok_or(ClientError::NoTransports)
            .and_then(|selected| {
                let url = Url::parse(&selected.url).map_err(|source| ClientError::Endpoint {
                    url: selected.url.clone(),
                    source,
                })?;
                Ok((url, selected.tenant))
            });
        let client = endpoint.map(|(url, tenant)| A2AClient {
            timeout: config.httpx_client.limits.read_timeout,
            http_client: config.httpx_client,
            call_context: None,
            agent_card: card.clone(),
            streaming: config.streaming,
            endpoint: url,
            tenant,
            request_ids: Arc::new(Mutex::new(Box::new(UuidRequestIds))),
        });
        ready(client)
    }

    fn attach_stash(
        &self,
        client: &mut A2AClient,
        stash: ClientStash<ProxiedClient, ClientCallContext, AgentCard>,
    ) {
        client.http_client = stash.httpx_client;
        client.call_context = stash.call_context;
        client.agent_card = stash.agent_card;
    }
}
