use async_trait::async_trait;
use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::agent_registry::{AgentRegistry, AgentRegistryApi as _};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_record::MCPServer;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::custom_url::CustomUrls;
use betterlite_server::public_endpoints::resource_hubs::agent_hub::public_agent_cards;
use betterlite_server::public_endpoints::resource_hubs::error::ResourceHubsError;
use betterlite_server::public_endpoints::resource_hubs::mcp_hub::public_mcp_servers as build_mcp_servers;
use betterlite_server::public_endpoints::resource_hubs::plugin_table::{PluginRow, PluginTable};
use betterlite_server::public_endpoints::resource_hubs::skill_hub::plugin_response;
use serde::Deserialize;

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    AgentHub {
        agents: Vec<AgentResponse>,
        #[serde(default)]
        public_agent_groups: Option<Vec<String>>,
        request_base_url: String,
        root_path: String,
        #[serde(default)]
        proxy_base_url: Option<String>,
    },
    McpHub {
        config_servers: Vec<MCPServer>,
        db_servers: Vec<MCPServer>,
        #[serde(default)]
        public_mcp_servers: Option<Vec<String>>,
        public_hub_strict: bool,
    },
    SkillHub {
        plugins: Vec<PluginRow>,
        #[serde(default)]
        table_error: bool,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::AgentHub {
                agents,
                public_agent_groups,
                request_base_url,
                root_path,
                proxy_base_url,
            } => {
                let registry = agent_registry(agents);
                let urls = CustomUrls {
                    proxy_base_url: proxy_base_url.clone(),
                };
                let cards = public_agent_cards(
                    &registry,
                    public_agent_groups.as_deref(),
                    &urls,
                    request_base_url,
                    root_path,
                )
                .map_err(hub_error)?;
                Ok(serde_json::to_value(cards)?)
            }
            Input::McpHub {
                config_servers,
                db_servers,
                public_mcp_servers,
                public_hub_strict,
            } => {
                let registry = ServerRegistry::new(
                    RegistrySettings {
                        public_mcp_servers: public_mcp_servers.clone().unwrap_or_default(),
                        public_hub_strict: *public_hub_strict,
                        internal_ip_ranges: None,
                    },
                    config_servers.clone(),
                    db_servers.clone(),
                );
                let entries = build_mcp_servers(&registry, public_mcp_servers.as_deref())
                    .map_err(hub_error)?;
                Ok(serde_json::to_value(entries)?)
            }
            Input::SkillHub {
                plugins,
                table_error,
            } => {
                let table = InputTable {
                    rows: plugins,
                    fail: *table_error,
                };
                let runtime = tokio::runtime::Builder::new_current_thread().build()?;
                let response = runtime
                    .block_on(plugin_response(&table))
                    .map_err(hub_error)?;
                Ok(serde_json::to_value(response)?)
            }
        }
    }
}

fn agent_registry(records: &[AgentResponse]) -> AgentRegistry {
    let registry = AgentRegistry::default();
    for record in records {
        registry.register_agent(record.clone());
    }
    registry
}

struct InputTable<'rows> {
    rows: &'rows [PluginRow],
    fail: bool,
}

#[async_trait]
impl PluginTable for InputTable<'_> {
    async fn enabled_plugins(&self) -> Result<Vec<PluginRow>, TableError> {
        if self.fail {
            return Err(TableError::MissingRow);
        }
        Ok(self.rows.to_vec())
    }
}

fn hub_error(error: ResourceHubsError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("HTTPException"),
        status: Some(500),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}
