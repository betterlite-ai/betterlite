use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::{HeaderMap, Method};
use url::Url;

use crate::agent_card::AgentCard;
use crate::card_resolver::fetch_error::{CardFetchError, ProbeFailure};
use crate::errors::A2AError;

pub mod card_url;
pub mod fetch_error;
pub mod interfaces;
pub mod localhost;
mod tests;

const AGENT_CARD_PATH: &str = "/.well-known/agent-card.json";
const WELL_KNOWN_CARD_PATHS: [&str; 3] = [
    AGENT_CARD_PATH,
    "/.well-known/agent.json",
    "/agentCard/v1.0",
];

pub struct CardResolver {
    pub base_url: String,
    pub client: ProxiedClient,
}

impl CardResolver {
    pub async fn get_agent_card(
        &self,
        relative_card_path: Option<&str>,
        extra_headers: &HeaderMap,
    ) -> Result<AgentCard, A2AError> {
        // An empty path is what the a2a-sdk resolver reads as "use the default card path".
        let paths = match relative_card_path {
            Some("") => vec![AGENT_CARD_PATH],
            Some(path) => vec![path],
            None => WELL_KNOWN_CARD_PATHS.to_vec(),
        };
        let mut failures = Vec::new();
        for path in paths {
            match self.fetch_card(path, extra_headers).await {
                Ok(card) => return Ok(card),
                Err(reason) => failures.push(ProbeFailure {
                    path: path.to_owned(),
                    reason,
                }),
            }
        }
        Err(A2AError::agent_card_discovery(
            self.base_url.clone(),
            failures,
        ))
    }

    async fn fetch_card(
        &self,
        path: &str,
        extra_headers: &HeaderMap,
    ) -> Result<AgentCard, CardFetchError> {
        let relative = path.trim_start_matches('/');
        let target = if relative.is_empty() {
            self.base_url.clone()
        } else {
            format!("{}/{relative}", self.base_url.trim_end_matches('/'))
        };
        let url = Url::parse(&target).map_err(CardFetchError::InvalidUrl)?;
        let request = OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers: extra_headers.clone(),
            body: RequestBody::Empty,
            deadline: self.client.limits.read_timeout,
        };
        let response = self
            .client
            .send_buffered(request)
            .await
            .map_err(CardFetchError::Transport)?;
        if response.status >= 400 {
            return Err(CardFetchError::Status {
                status: response.status,
            });
        }
        serde_json::from_slice::<AgentCard>(&response.body).map_err(CardFetchError::Parse)
    }
}
