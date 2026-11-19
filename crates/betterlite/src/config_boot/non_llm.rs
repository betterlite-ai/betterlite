use std::path::Path;

use betterlite_config::sections::includes::ConfigMap;
use betterlite_config::sections::settings::JsonValue;

use super::error::ConfigBootError;
use super::raw::is_configured;
use crate::job_scheduler::BoxFuture;

// The registries behind these methods live in other modules; the steps only hand them config
pub trait NonLlmRegistries: Send + Sync {
    fn mcp_tools<'config>(
        &'config self,
        tools: &'config JsonValue,
        config_file_path: Option<&'config Path>,
    ) -> BoxFuture<'config, Result<(), ConfigBootError>>;

    fn agents<'config>(
        &'config self,
        agents: &'config JsonValue,
    ) -> BoxFuture<'config, Result<(), ConfigBootError>>;

    fn mcp_servers<'config>(
        &'config self,
        servers: &'config JsonValue,
        mcp_aliases: Option<&'config JsonValue>,
    ) -> BoxFuture<'config, Result<(), ConfigBootError>>;

    fn vector_stores<'config>(
        &'config self,
        registry: &'config JsonValue,
    ) -> BoxFuture<'config, Result<(), ConfigBootError>>;
}

pub async fn load_non_llm_configs(
    config: &ConfigMap,
    config_file_path: Option<&Path>,
    ports: &dyn NonLlmRegistries,
) -> Result<(), ConfigBootError> {
    if let Some(tools) = config.get("mcp_tools")
        && is_configured(tools)
    {
        ports.mcp_tools(tools, config_file_path).await?;
    }
    let configured_agents = config.get("agents").or_else(|| config.get("agent_list"));
    if let Some(agents) = configured_agents
        && !agents.is_null()
    {
        ports.agents(agents).await?;
    }
    if let Some(servers) = config.get("mcp_servers")
        && is_configured(servers)
    {
        let aliases = mcp_aliases(config)?;
        ports.mcp_servers(servers, aliases).await?;
    }
    if let Some(registry) = config.get("vector_store_registry")
        && is_configured(registry)
    {
        ports.vector_stores(registry).await?;
    }
    if let Some(workers) = config.get("worker_registry")
        && is_configured(workers)
    {
        return Err(ConfigBootError::WorkerRegistryNeedsPremium);
    }
    Ok(())
}

fn mcp_aliases(config: &ConfigMap) -> Result<Option<&JsonValue>, ConfigBootError> {
    let Some(settings) = config.get("litellm_settings") else {
        return Ok(None);
    };
    match settings {
        JsonValue::Object(fields) => Ok(fields.get("mcp_aliases")),
        JsonValue::Null => Ok(None),
        JsonValue::Array(_) | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => {
            Err(ConfigBootError::FieldType {
                field: "litellm_settings",
                expected: "a mapping",
            })
        }
    }
}

mod tests;
