use std::time::Duration;

use betterlite_core::agents::{AgentKillSwitchConfig, AgentKillSwitchResult};
use thiserror::Error;

use super::client::{KillSwitchHttpClient, KillSwitchResponse, KillSwitchSendOptions};
use super::request::{KillSwitchRequest, build_kill_switch_request, method_text};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
pub const RESPONSE_BODY_MAX_CHARS: usize = 2000;

#[derive(Debug, Error)]
enum DeliveryFailure {
    #[error("the kill switch client could not build the request: {message}")]
    Build { message: String },
    #[error("the kill switch request failed: {message}")]
    Send { message: String },
    #[error("the kill switch response failed: {message}")]
    Response { message: String },
    #[error("the kill switch response could not be released: {message}")]
    Close { message: String },
}

impl DeliveryFailure {
    fn message(self) -> String {
        match self {
            Self::Build { message }
            | Self::Send { message }
            | Self::Response { message }
            | Self::Close { message } => message,
        }
    }
}

#[must_use]
pub async fn fire_kill_switch<Client>(
    agent_id: &str,
    config: &AgentKillSwitchConfig,
    http_client: &Client,
    timeout: Duration,
) -> AgentKillSwitchResult
where
    Client: KillSwitchHttpClient,
{
    match build_kill_switch_request(config) {
        Ok(request) => {
            tracing::info!(
                "Firing kill switch for agent {agent_id}: {} {}",
                method_text(&request.method),
                request.reported_url
            );
            deliver(agent_id, request, http_client, timeout).await
        }
        Err(error) => AgentKillSwitchResult {
            agent_id: String::from(agent_id),
            url: config.url.clone(),
            method: config.method.clone(),
            status_code: None,
            response_body: None,
            error: Some(error.to_string()),
        },
    }
}

async fn deliver<Client>(
    agent_id: &str,
    request: KillSwitchRequest,
    http_client: &Client,
    timeout: Duration,
) -> AgentKillSwitchResult
where
    Client: KillSwitchHttpClient,
{
    let (status_code, response_body, error) = match exchange(http_client, &request, timeout).await {
        Ok((status_code, body)) => (Some(status_code), Some(body), None),
        Err(failure) => {
            let message = failure.message();
            tracing::warn!("Kill switch for agent {agent_id} failed: {message}");
            (None, None, Some(message))
        }
    };
    AgentKillSwitchResult {
        agent_id: String::from(agent_id),
        url: request.reported_url,
        method: request.method,
        status_code,
        response_body,
        error,
    }
}

async fn exchange<Client>(
    http_client: &Client,
    request: &KillSwitchRequest,
    timeout: Duration,
) -> Result<(u16, String), DeliveryFailure>
where
    Client: KillSwitchHttpClient,
{
    let built = http_client
        .build_request(request, timeout)
        .map_err(|error| DeliveryFailure::Build {
            message: error.to_string(),
        })?;
    let mut response = http_client
        .send(
            built,
            KillSwitchSendOptions {
                stream: true,
                follow_redirects: false,
            },
        )
        .await
        .map_err(|error| DeliveryFailure::Send {
            message: error.to_string(),
        })?;
    let status_code = response.status();
    let body = read_prefix(&mut response).await;
    // the response goes back to the pool on every path, as httpx's finally aclose does
    response
        .close()
        .await
        .map_err(|error| DeliveryFailure::Close {
            message: error.to_string(),
        })?;
    Ok((status_code, body?))
}

async fn read_prefix<Response>(response: &mut Response) -> Result<String, DeliveryFailure>
where
    Response: KillSwitchResponse,
{
    let mut text = String::new();
    let mut length = 0_usize;
    while length < RESPONSE_BODY_MAX_CHARS {
        let Some(chunk) = response.next_text_chunk().await else {
            break;
        };
        let piece = chunk.map_err(|error| DeliveryFailure::Response {
            message: error.to_string(),
        })?;
        // a chunk with no text ends the body, so a client that keeps yielding empty ones cannot hang the reader
        if piece.is_empty() {
            break;
        }
        length = length.saturating_add(piece.chars().count());
        text.push_str(&piece);
    }
    Ok(text.chars().take(RESPONSE_BODY_MAX_CHARS).collect())
}

mod tests;
