pub mod agent_hub;
pub mod error;
pub mod mcp_hub;
pub mod plugin_table;
pub mod plugin_types;
pub mod skill_hub;
mod tests;
mod timestamps;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use betterlite_auth::agent_registry::AgentRegistry;
use betterlite_mcp::registry::servers::SharedRegistry;

use self::agent_hub::agent_hub;
use self::mcp_hub::mcp_hub;
use self::plugin_table::PluginTable;
use self::skill_hub::skill_hub;
use crate::agent_endpoints::admin_actions::ports::PublicGroupStore;
use crate::custom_url::CustomUrls;

pub const AGENT_HUB_PATH: &str = "/public/agent_hub";
pub const MCP_HUB_PATH: &str = "/public/mcp_hub";
pub const SKILL_HUB_PATH: &str = "/public/skill_hub";

#[derive(Clone)]
pub struct ResourceHubsState {
    pub agents: Arc<AgentRegistry>,
    pub public_agent_groups: Arc<dyn PublicGroupStore>,
    pub mcp: SharedRegistry,
    pub public_mcp_servers: Option<Vec<String>>,
    pub plugins: Option<Arc<dyn PluginTable>>,
    pub custom_urls: CustomUrls,
}

pub fn resource_hubs_routes(state: ResourceHubsState) -> Router {
    Router::new()
        .route(AGENT_HUB_PATH, get(agent_hub))
        .route(MCP_HUB_PATH, get(mcp_hub))
        .route(SKILL_HUB_PATH, get(skill_hub))
        .with_state(state)
}
