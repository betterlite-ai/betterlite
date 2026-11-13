use std::error::Error;
use std::fmt::Display;
use std::future::Future;

use thiserror::Error;

mod tests;

pub const CONNECTION_ERROR_PATTERNS: [&str; 4] = ["connect", "connection", "network", "refused"];

pub const AGENT_CARD_ERROR_PATTERNS: [&str; 5] = [
    "agent card",
    "agent-card",
    ".well-known",
    "card not found",
    "invalid agent",
];

// Card helpers live in betterlite-a2a (B4800), which depends on this crate, so they come in through this trait.
pub trait CardUrls {
    type Card: Send;

    fn is_localhost_or_internal_url(&self, url: Option<&str>) -> bool;

    fn set_agent_card_url(&self, card: &mut Self::Card, url: &str);
}

// The client and its A2A SDK calls belong to B4801 and the SDK, so recovery reaches them through this trait.
pub trait A2aSdk: CardUrls + Sync {
    const A2A_SDK_AVAILABLE: bool;

    type Client: Sync;
    type HttpClient: Clone + Send;
    type CallContext;
    type SdkError: Error + 'static;

    // Reads the client LiteLLM stashes as _litellm_httpx_client; None for a client built elsewhere.
    fn httpx_client(&self, client: &Self::Client) -> Option<Self::HttpClient>;

    // Reads the stash LiteLLM keeps as _litellm_call_context; None when the client carries none.
    fn call_context(&self, client: &Self::Client) -> Option<Self::CallContext>;

    fn create_client(
        &self,
        card: &Self::Card,
        config: ClientConfig<Self::HttpClient>,
    ) -> impl Future<Output = Result<Self::Client, Self::SdkError>> + Send;

    // Writes the _litellm_httpx_client, _litellm_call_context and _litellm_agent_card stashes.
    fn attach_stash(
        &self,
        client: &mut Self::Client,
        stash: ClientStash<Self::HttpClient, Self::CallContext, Self::Card>,
    );
}

pub struct ClientConfig<HttpClient> {
    pub httpx_client: HttpClient,
    pub streaming: bool,
}

pub struct ClientStash<HttpClient, CallContext, Card> {
    pub httpx_client: HttpClient,
    pub call_context: Option<CallContext>,
    pub agent_card: Card,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("the agent card points at the internal URL '{localhost_url}'; retry with '{base_url}'")]
pub struct LocalhostUrlError {
    pub localhost_url: String,
    pub base_url: String,
    pub original_error: String,
    pub model: Option<String>,
}

// The variants mirror LiteLLM's A2ALocalhostURLError, A2AAgentCardError, A2AConnectionError and A2AError.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum A2AException {
    #[error(transparent)]
    LocalhostUrl(LocalhostUrlError),
    #[error("{message}")]
    AgentCard {
        message: String,
        url: Option<String>,
        model: Option<String>,
    },
    #[error("{message}")]
    Connection {
        message: String,
        url: Option<String>,
        model: Option<String>,
    },
    #[error("{message}")]
    Other {
        message: String,
        model: Option<String>,
    },
}

#[derive(Debug, Error)]
pub enum RecoveryError<SdkError> {
    #[error("A2A SDK is required for localhost retry handling")]
    SdkUnavailable,
    #[error(
        "cannot retry the A2A localhost URL fix: no agent card is available to rewrite, so the upstream URL cannot be corrected"
    )]
    MissingAgentCard,
    #[error(
        "cannot retry the A2A localhost URL fix: the client was not created by create_a2a_client, so no httpx client is attached"
    )]
    ExternalClient,
    #[error("the A2A SDK could not create the retry client")]
    CreateClient(#[source] SdkError),
}

pub struct A2AExceptionCheckers;

impl A2AExceptionCheckers {
    #[must_use]
    pub fn is_connection_error(error: &str) -> bool {
        let lowered = error.to_lowercase();
        CONNECTION_ERROR_PATTERNS
            .iter()
            .any(|pattern| lowered.contains(pattern))
    }

    #[must_use]
    pub fn is_agent_card_error(error: &str) -> bool {
        let lowered = error.to_lowercase();
        AGENT_CARD_ERROR_PATTERNS
            .iter()
            .any(|pattern| lowered.contains(pattern))
    }
}

// The first match wins: a localhost connection failure must come out as the retryable variant.
#[must_use]
pub fn map_a2a_exception<Cards, Original>(
    cards: &Cards,
    original_error: &Original,
    card_url_hint: Option<&str>,
    api_base_hint: Option<&str>,
    model_name: Option<&str>,
) -> A2AException
where
    Cards: CardUrls,
    Original: Display,
{
    let message = original_error.to_string();
    let model = model_name.map(str::to_owned);
    if let (Some(card_url), Some(api_base)) = (card_url_hint, api_base_hint)
        && cards.is_localhost_or_internal_url(Some(card_url))
        && A2AExceptionCheckers::is_connection_error(&message)
    {
        return A2AException::LocalhostUrl(LocalhostUrlError {
            localhost_url: card_url.to_owned(),
            base_url: api_base.to_owned(),
            original_error: message,
            model,
        });
    }
    if A2AExceptionCheckers::is_agent_card_error(&message) {
        return A2AException::AgentCard {
            message,
            url: api_base_hint.map(str::to_owned),
            model,
        };
    }
    if A2AExceptionCheckers::is_connection_error(&message) {
        return A2AException::Connection {
            message,
            url: card_url_hint.or(api_base_hint).map(str::to_owned),
            model,
        };
    }
    A2AException::Other { message, model }
}

pub async fn handle_a2a_localhost_retry<Sdk>(
    sdk: &Sdk,
    error: &LocalhostUrlError,
    card_to_fix: Option<&mut Sdk::Card>,
    a2a_client: &Sdk::Client,
    is_streaming: bool,
) -> Result<Sdk::Client, RecoveryError<Sdk::SdkError>>
where
    Sdk: A2aSdk,
    Sdk::Card: Clone,
{
    if !Sdk::A2A_SDK_AVAILABLE {
        return Err(RecoveryError::SdkUnavailable);
    }
    let agent_card = card_to_fix.ok_or(RecoveryError::MissingAgentCard)?;
    let request_type = if is_streaming { "streaming " } else { "" };
    tracing::warn!(
        "A2A {request_type}request to '{}' failed: {}. Agent card contains localhost/internal URL. Retrying with base_url '{}'.",
        error.localhost_url,
        error.original_error,
        error.base_url,
    );
    sdk.set_agent_card_url(agent_card, &error.base_url);
    let httpx_client = sdk
        .httpx_client(a2a_client)
        .ok_or(RecoveryError::ExternalClient)?;
    let config = ClientConfig {
        httpx_client: httpx_client.clone(),
        streaming: is_streaming,
    };
    let mut new_client = sdk
        .create_client(agent_card, config)
        .await
        .map_err(RecoveryError::CreateClient)?;
    let stash = ClientStash {
        httpx_client,
        call_context: sdk.call_context(a2a_client),
        agent_card: agent_card.clone(),
    };
    sdk.attach_stash(&mut new_client, stash);
    Ok(new_client)
}
