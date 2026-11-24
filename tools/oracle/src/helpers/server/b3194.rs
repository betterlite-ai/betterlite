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
use tokio::runtime::Builder;

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "agent_hub")]
    Agent {
        agents: Vec<AgentResponse>,
        #[serde(default)]
        public_agent_groups: Option<Vec<String>>,
        request_base_url: String,
        root_path: String,
        #[serde(default)]
        proxy_base_url: Option<String>,
    },
    #[serde(rename = "mcp_hub")]
    Mcp {
        config_servers: Vec<MCPServer>,
        db_servers: Vec<MCPServer>,
        #[serde(default)]
        public_mcp_servers: Option<Vec<String>>,
        public_hub_strict: bool,
    },
    #[serde(rename = "skill_hub")]
    Skill {
        plugins: Vec<PluginRow>,
        #[serde(default)]
        table_error: bool,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Agent {
                agents,
                public_agent_groups,
                request_base_url,
                root_path,
                proxy_base_url,
            } => agent_hub_answer(
                agents,
                public_agent_groups.as_deref(),
                request_base_url,
                root_path,
                proxy_base_url.as_deref(),
            ),
            Input::Mcp {
                config_servers,
                db_servers,
                public_mcp_servers,
                public_hub_strict,
            } => mcp_hub_answer(
                config_servers,
                db_servers,
                public_mcp_servers.as_deref(),
                *public_hub_strict,
            ),
            Input::Skill {
                plugins,
                table_error,
            } => skill_hub_answer(plugins, *table_error),
        }
    }
}

fn agent_hub_answer(
    agents: &[AgentResponse],
    public_agent_groups: Option<&[String]>,
    request_base_url: &str,
    root_path: &str,
    proxy_base_url: Option<&str>,
) -> HelperResult {
    let registry = agent_registry(agents);
    let urls = CustomUrls {
        proxy_base_url: proxy_base_url.map(String::from),
    };
    let cards = public_agent_cards(
        &registry,
        public_agent_groups,
        &urls,
        request_base_url,
        root_path,
    )
    .map_err(|error| hub_error(&error))?;
    Ok(serde_json::to_value(cards)?)
}

fn mcp_hub_answer(
    config_servers: &[MCPServer],
    db_servers: &[MCPServer],
    public_mcp_servers: Option<&[String]>,
    public_hub_strict: bool,
) -> HelperResult {
    let registry = ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: public_mcp_servers.unwrap_or(&[]).to_vec(),
            public_hub_strict,
            internal_ip_ranges: None,
        },
        config_servers.to_vec(),
        db_servers.to_vec(),
    );
    let entries =
        build_mcp_servers(&registry, public_mcp_servers).map_err(|error| hub_error(&error))?;
    Ok(serde_json::to_value(entries)?)
}

fn skill_hub_answer(plugins: &[PluginRow], table_error: bool) -> HelperResult {
    let table = InputTable {
        rows: plugins,
        fail: table_error,
    };
    let runtime = Builder::new_current_thread().build()?;
    let response = runtime
        .block_on(plugin_response(&table))
        .map_err(|error| hub_error(&error))?;
    Ok(serde_json::to_value(response)?)
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

fn hub_error(error: &ResourceHubsError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("HTTPException"),
        status: Some(500),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}
