use std::path::Path;
use std::sync::{Arc, PoisonError, RwLock};

use betterlite_config::sections::mcp_tools::{McpToolEntry, McpToolSettings};
use betterlite_models::decisions::Json as ConfigJson;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use thiserror::Error;
use tracing::{debug, warn};

use crate::tool_registry::handler::{LocalTool, ToolHandler};
use crate::tool_registry::loader::{HandlerLoadError, ToolHandlerLoader};
use crate::tools::schemas::McpTool;

#[derive(Default)]
pub struct ToolRegistry {
    tools: RwLock<IndexMap<String, LocalTool>>,
}

#[derive(Debug, Error)]
pub enum ToolRegistryError {
    #[error("mcp_tools_config is required, please set `mcp_tools` in your proxy config")]
    Missing,
    #[error("mcp_tools_config must be a list of dictionaries")]
    NotAMapping,
    #[error(transparent)]
    Handler(#[from] HandlerLoadError),
}

impl ToolRegistry {
    pub fn register_tool(
        &self,
        name: &str,
        description: &str,
        input_schema: Object,
        handler: Arc<dyn ToolHandler>,
    ) {
        self.tools
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                name.to_owned(),
                LocalTool {
                    name: name.to_owned(),
                    description: description.to_owned(),
                    input_schema,
                    handler,
                },
            );
        debug!("Registered tool: {name}");
    }

    #[must_use]
    pub fn get_tool(&self, name: &str) -> Option<LocalTool> {
        self.tools
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
    }

    #[must_use]
    pub fn list_tools(&self, tool_prefix: Option<&str>) -> Vec<LocalTool> {
        let tools = self.tools.read().unwrap_or_else(PoisonError::into_inner);
        let Some(prefix) = tool_prefix.filter(|prefix| !prefix.is_empty()) else {
            return tools.values().cloned().collect();
        };
        tools
            .values()
            .filter(|tool| tool.name.starts_with(prefix))
            .cloned()
            .collect()
    }

    #[must_use]
    pub fn unregister_tools_with_prefix(&self, prefix: &str) -> usize {
        if prefix.is_empty() {
            return 0;
        }
        let mut tools = self.tools.write().unwrap_or_else(PoisonError::into_inner);
        let before = tools.len();
        tools.retain(|name, _| {
            if name.starts_with(prefix) {
                debug!("Unregistered MCP tool {name}");
                return false;
            }
            true
        });
        before.saturating_sub(tools.len())
    }

    #[must_use]
    pub fn to_sdk_tools(tools: &[LocalTool]) -> Vec<McpTool> {
        tools
            .iter()
            .map(|tool| McpTool {
                name: tool.name.clone(),
                description: Some(tool.description.clone()),
                input_schema: Some(tool.input_schema.clone()),
                extra: Object::new(),
            })
            .collect()
    }

    pub fn load_tools_from_config(
        &self,
        entries: Option<&[McpToolEntry]>,
        config_file_path: Option<&Path>,
        loader: &dyn ToolHandlerLoader,
    ) -> Result<(), ToolRegistryError> {
        let Some(configured) = entries else {
            return Err(ToolRegistryError::Missing);
        };
        for entry in configured {
            let McpToolEntry::Tool(settings) = entry else {
                return Err(ToolRegistryError::NotAMapping);
            };
            let Some((name, description, handler_name)) = complete(settings) else {
                continue;
            };
            let Some(handler) = loader.load(handler_name, config_file_path)? else {
                warn!("Warning: Could not find handler {handler_name} for tool {name}");
                continue;
            };
            self.register_tool(
                name,
                description,
                prompt_object(&settings.input_schema),
                handler,
            );
        }
        let names: Vec<String> = self
            .list_tools(None)
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        debug!("all registered tools: {names:?}");
        Ok(())
    }
}

// Python's all([name, description, handler]): a missing, null or empty string skips the entry
fn complete(settings: &McpToolSettings) -> Option<(&str, &str, &str)> {
    let name = settings.name.as_deref().filter(|name| !name.is_empty())?;
    let description = settings
        .description
        .as_deref()
        .filter(|description| !description.is_empty())?;
    let handler = settings
        .handler
        .as_deref()
        .filter(|handler| !handler.is_empty())?;
    Some((name, description, handler))
}

fn prompt_object(fields: &IndexMap<String, ConfigJson>) -> Object {
    let mut object = Object::with_capacity(fields.len());
    for (key, value) in fields {
        object.insert(key.clone(), prompt_json(value));
    }
    object
}

fn prompt_json(value: &ConfigJson) -> Json {
    match value {
        ConfigJson::Null => Json::Null,
        ConfigJson::Bool(flag) => Json::Bool(*flag),
        ConfigJson::Number(number) => Json::Number(number.clone()),
        ConfigJson::Text(text) => Json::Text(text.clone()),
        ConfigJson::List(items) => Json::List(items.iter().map(prompt_json).collect()),
        ConfigJson::Object(fields) => Json::Object(prompt_object(fields)),
    }
}
