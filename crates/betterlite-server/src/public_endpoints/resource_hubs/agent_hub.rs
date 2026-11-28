use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Request, State};
use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::agent_registry::AgentRegistryApi;
use betterlite_core::agents::AgentCard;
use serde_json::value::RawValue;

use super::ResourceHubsState;
use super::error::ResourceHubsError;
use crate::custom_url::CustomUrls;
use crate::middleware::request_root_paths::{get_request_root_path, request_base_url};

// Every field AgentCard declares, so a key pydantic would drop never reaches its deny_unknown_fields parser.
const AGENT_CARD_KEYS: [&str; 18] = [
    "protocolVersion",
    "name",
    "description",
    "url",
    "version",
    "capabilities",
    "defaultInputModes",
    "defaultOutputModes",
    "skills",
    "preferredTransport",
    "additionalInterfaces",
    "iconUrl",
    "provider",
    "documentationUrl",
    "securitySchemes",
    "security",
    "supportsAuthenticatedExtendedCard",
    "signatures",
];

pub async fn agent_hub(
    State(state): State<ResourceHubsState>,
    request: Request,
) -> Result<Json<Vec<AgentCard>>, ResourceHubsError> {
    let root_path = get_request_root_path(request.extensions(), "");
    let base_url = request_base_url(&request, &root_path);
    let cards = public_agent_cards(
        state.agents.as_ref(),
        state.public_agent_groups.groups().as_deref(),
        &state.custom_urls,
        &base_url,
        &root_path,
    )?;
    Ok(Json(cards))
}

pub fn public_agent_cards<R>(
    registry: &R,
    groups: Option<&[String]>,
    custom_urls: &CustomUrls,
    base_url: &str,
    root_path: &str,
) -> Result<Vec<AgentCard>, ResourceHubsError>
where
    R: AgentRegistryApi,
{
    let Some(allowed) = groups else {
        return Ok(Vec::new());
    };
    let mut cards = Vec::new();
    for agent in registry.get_public_agent_list(Some(allowed)) {
        cards.push(public_agent_card(&agent, custom_urls, base_url, root_path)?);
    }
    Ok(cards)
}

fn public_agent_card(
    agent: &AgentResponse,
    custom_urls: &CustomUrls,
    base_url: &str,
    root_path: &str,
) -> Result<AgentCard, ResourceHubsError> {
    let mut params: BTreeMap<String, Box<RawValue>> = agent
        .agent_card_params
        .iter()
        .filter(|(key, _)| AGENT_CARD_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let route = format!("a2a/{}", agent.agent_id);
    let url = custom_urls.custom_route_url(base_url, root_path, &route);
    let encoded = serde_json::to_string(&url).map_err(ResourceHubsError::AgentCard)?;
    params.insert(
        String::from("url"),
        RawValue::from_string(encoded).map_err(ResourceHubsError::AgentCard)?,
    );
    let body = serde_json::to_string(&params).map_err(ResourceHubsError::AgentCard)?;
    serde_json::from_str(&body).map_err(ResourceHubsError::AgentCard)
}
