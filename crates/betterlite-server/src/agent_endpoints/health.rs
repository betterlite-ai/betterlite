use std::time::Duration;

use async_trait::async_trait;
use axum::http::{HeaderMap, Method};
use betterlite_config::sections::agent_search::Settings as AgentSearchSettings;
use betterlite_core::agents::AgentResponse;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use futures_util::future::join_all;
use thiserror::Error;
use tokio::time::timeout;
use url::Url;

mod tests;

const TIMED_OUT_ERROR: &str = "Health check timed out";
const SERVER_ERROR_STATUS: u16 = 500;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentHealthResult {
    pub agent_id: String,
    pub healthy: bool,
    pub error: Option<String>,
}

#[derive(Debug, Error)]
pub enum AgentHealthError {
    #[error("the agent URL is not a valid URL")]
    Url(#[source] url::ParseError),
    #[error("the health check request failed")]
    Request(#[source] TransportError),
    #[error("{message}")]
    Unreachable { message: String },
}

#[async_trait]
pub trait AgentHealthTransport: Send + Sync {
    async fn get_status(&self, url: &str, timeout: Duration) -> Result<u16, AgentHealthError>;
}

pub struct PooledHealthTransport {
    pub client: ProxiedClient,
}

#[async_trait]
impl AgentHealthTransport for PooledHealthTransport {
    async fn get_status(&self, url: &str, timeout: Duration) -> Result<u16, AgentHealthError> {
        let target = Url::parse(url).map_err(AgentHealthError::Url)?;
        let request = OutboundRequest {
            method: Method::GET,
            url: target,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Empty,
            deadline: timeout,
        };
        let response = self
            .client
            .send_following(request)
            .await
            .map_err(AgentHealthError::Request)?;
        Ok(response.response.status().as_u16())
    }
}

pub async fn check_agent_url_health(
    agent: &AgentResponse,
    transport: &dyn AgentHealthTransport,
    timeout: Duration,
) -> AgentHealthResult {
    let url = agent
        .agent_card_params
        .url
        .as_deref()
        .filter(|url| !url.is_empty());
    let Some(target) = url else {
        return AgentHealthResult {
            agent_id: agent.agent_id.clone(),
            healthy: true,
            error: None,
        };
    };
    match transport.get_status(target, timeout).await {
        Ok(status) if status >= SERVER_ERROR_STATUS => AgentHealthResult {
            agent_id: agent.agent_id.clone(),
            healthy: false,
            error: Some(format!("HTTP {status}")),
        },
        Ok(_) => AgentHealthResult {
            agent_id: agent.agent_id.clone(),
            healthy: true,
            error: None,
        },
        Err(error) => AgentHealthResult {
            agent_id: agent.agent_id.clone(),
            healthy: false,
            error: Some(error.to_string()),
        },
    }
}

pub async fn filter_healthy_agents(
    agents: Vec<AgentResponse>,
    transport: &dyn AgentHealthTransport,
    settings: &AgentSearchSettings,
) -> Vec<AgentResponse> {
    let (with_url, without_url): (Vec<AgentResponse>, Vec<AgentResponse>) =
        agents.into_iter().partition(|agent| {
            agent
                .agent_card_params
                .url
                .as_deref()
                .is_some_and(|url| !url.is_empty())
        });
    let results = probe_agents(&with_url, transport, settings).await;
    with_url
        .into_iter()
        .zip(results)
        .filter_map(|(agent, result)| result.healthy.then_some(agent))
        .chain(without_url)
        .collect()
}

async fn probe_agents(
    agents: &[AgentResponse],
    transport: &dyn AgentHealthTransport,
    settings: &AgentSearchSettings,
) -> Vec<AgentHealthResult> {
    let Ok(probe_timeout) = Duration::try_from_secs_f64(settings.health_check_timeout_seconds)
    else {
        return failed_results(agents, "the health check timeout is not a valid duration");
    };
    let Ok(gather_timeout) = Duration::try_from_secs_f64(settings.health_gather_timeout_seconds)
    else {
        tracing::warn!(
            "Agent health check gather timed out after {:?} seconds",
            settings.health_gather_timeout_seconds
        );
        return failed_results(agents, TIMED_OUT_ERROR);
    };
    let probes = agents
        .iter()
        .map(|agent| check_agent_url_health(agent, transport, probe_timeout));
    match timeout(gather_timeout, join_all(probes)).await {
        Ok(results) => results,
        Err(_elapsed) => {
            tracing::warn!(
                "Agent health check gather timed out after {:?} seconds",
                settings.health_gather_timeout_seconds
            );
            failed_results(agents, TIMED_OUT_ERROR)
        }
    }
}

fn failed_results(agents: &[AgentResponse], message: &str) -> Vec<AgentHealthResult> {
    agents
        .iter()
        .map(|agent| AgentHealthResult {
            agent_id: agent.agent_id.clone(),
            healthy: false,
            error: Some(message.to_owned()),
        })
        .collect()
}
