use std::path::Path;
use std::sync::Arc;

use betterlite_auth::agent_registry::AgentRegistry;
use betterlite_auth::agent_registry::AgentRegistryApi as _;
use betterlite_auth::agent_registry::AgentRegistryError;
use betterlite_config::sections::settings::JsonValue;
use betterlite_models::decisions::Json;

use crate::config_boot::error::ConfigBootError;
use crate::config_boot::non_llm::NonLlmRegistries;
use crate::job_scheduler::BoxFuture;

pub mod error;
mod tests;

use self::error::BootRegistryError;

pub struct BootRegistries {
    agents: Arc<AgentRegistry>,
}

impl BootRegistries {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions - B8613 builds the registry set through this fixed name
    pub fn new(agents: Arc<AgentRegistry>) -> Self {
        Self { agents }
    }
}

impl NonLlmRegistries for BootRegistries {
    fn mcp_tools<'config>(
        &'config self,
        _tools: &'config JsonValue,
        _config_file_path: Option<&'config Path>,
    ) -> BoxFuture<'config, Result<(), ConfigBootError>> {
        Box::pin(async { Err(not_built("mcp_tools")) })
    }

    fn agents<'config>(
        &'config self,
        agents: &'config JsonValue,
    ) -> BoxFuture<'config, Result<(), ConfigBootError>> {
        Box::pin(async move {
            let entries = agent_entries(agents)?;
            self.agents
                .load_agents_from_config(Some(&entries))
                .map_err(agents_step)
        })
    }

    fn mcp_servers<'config>(
        &'config self,
        _servers: &'config JsonValue,
        _mcp_aliases: Option<&'config JsonValue>,
    ) -> BoxFuture<'config, Result<(), ConfigBootError>> {
        Box::pin(async { Err(not_built("mcp_servers")) })
    }

    fn vector_stores<'config>(
        &'config self,
        _registry: &'config JsonValue,
    ) -> BoxFuture<'config, Result<(), ConfigBootError>> {
        Box::pin(async { Err(not_built("vector_store_registry")) })
    }
}

fn not_built(key: &'static str) -> ConfigBootError {
    ConfigBootError::Step {
        step: key,
        source: Box::new(BootRegistryError::NotBuilt { key }),
    }
}

fn agents_step(source: AgentRegistryError) -> ConfigBootError {
    ConfigBootError::Step {
        step: "agents",
        source: Box::new(source),
    }
}

fn agent_entries(config: &JsonValue) -> Result<Vec<Json>, ConfigBootError> {
    let JsonValue::Array(items) = config else {
        return Err(agents_step(AgentRegistryError::NotAListOfDictionaries));
    };
    Ok(items.iter().map(json_value).collect())
}

fn json_value(value: &JsonValue) -> Json {
    match value {
        JsonValue::Null => Json::Null,
        JsonValue::Bool(flag) => Json::Bool(*flag),
        JsonValue::Number(number) => Json::Number(number.clone()),
        JsonValue::String(text) => Json::Text(text.clone()),
        JsonValue::Array(items) => Json::List(items.iter().map(json_value).collect()),
        JsonValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, field)| (key.clone(), json_value(field)))
                .collect(),
        ),
    }
}
