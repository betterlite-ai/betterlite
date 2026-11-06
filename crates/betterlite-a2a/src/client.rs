use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_core::a2a::exception_mapping::{
    A2aSdk, CardUrls, ClientConfig, ClientStash, map_a2a_exception,
};
use betterlite_core::agents::json::AgentJson;
use betterlite_http::proxy::{ProxiedClient, limits::TransportLimits};
use secrecy::SecretString;
use url::Url;

use crate::agent_card::AgentCard;
use crate::card_resolver::{
    CardResolver, card_url, card_url::get_agent_card_url, interfaces::normalize_interfaces,
    localhost,
};
use crate::client::error::{ClientError, exception_to_a2a};
use crate::errors::A2AError;

pub mod convert;
pub mod error;
pub mod records;
pub mod retry;
pub mod send;
mod tests;
pub mod transport;

pub const DEFAULT_A2A_AGENT_TIMEOUT: Duration = Duration::from_mins(100);

#[derive(Clone, Debug, Default)]
pub struct ClientCallContext {
    pub service_parameters: Option<BTreeMap<String, SecretString>>,
}

#[derive(Clone, Debug)]
pub struct A2AClientSettings {
    pub base_url: String,
    pub timeout: Duration,
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
) -> Result<A2AClient, A2AError> {
    let effective = effective_client(http, settings.timeout);
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
            read_timeout: timeout,
            ..pool.limits
        },
        ..pool.clone()
    }
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
        let endpoint = get_agent_card_url(card)
            .ok_or(ClientError::MissingEndpoint)
            .and_then(|text| {
                Url::parse(text).map_err(|source| ClientError::Endpoint {
                    url: text.to_owned(),
                    source,
                })
            });
        let client = endpoint.map(|parsed| A2AClient {
            http_client: config.httpx_client,
            call_context: None,
            agent_card: card.clone(),
            streaming: config.streaming,
            endpoint: parsed,
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
