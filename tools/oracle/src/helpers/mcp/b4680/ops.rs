use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_config::sections::mcp_tools::decode_mcp_tools;
use betterlite_mcp::openapi_tools::output::OpenApiToolOutput;
use betterlite_mcp::openapi_tools::request_headers::OpenApiCallContext;
use betterlite_mcp::tool_registry::handler::{ToolHandler, ToolHandlerError};
use betterlite_mcp::tool_registry::loader::{HandlerLoadError, ToolHandlerLoader};
use betterlite_mcp::tool_registry::registry::ToolRegistry;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use serde_json::{Value, json};

use super::input::{Function, Input};

pub(crate) fn run(input: &Input) -> HelperResult {
    let registry = ToolRegistry::default();
    for name in &input.tool_names {
        registry.register_tool(name, "", Object::new(), Arc::new(Silent));
    }
    match input.function {
        Function::ListTools => {
            let tools = registry.list_tools(input.prefix.as_deref());
            let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
            Ok(json!({"names": names, "removed": 0}))
        }
        Function::UnregisterPrefix => {
            let prefix = input.prefix.as_deref().unwrap_or_default();
            let removed = registry.unregister_tools_with_prefix(prefix);
            let tools = registry.list_tools(None);
            let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
            Ok(json!({"names": names, "removed": removed}))
        }
        Function::LoadFromConfig => load(&registry, input),
    }
}

// LiteLLM raises on a missing or non-list config, so the Python side answers the message as a value
fn load(registry: &ToolRegistry, input: &Input) -> HelperResult {
    let outcome = match &input.mcp_tools {
        None => registry.load_tools_from_config(None, None, &LenLoader),
        Some(configured) => {
            let bytes = serde_json::to_vec(&json!({"mcp_tools": configured}))?;
            let section = decode_mcp_tools(&bytes).or_value_error()?;
            registry.load_tools_from_config(Some(section.entries.as_slice()), None, &LenLoader)
        }
    };
    if let Err(error) = outcome {
        return Ok(json!({"error": error.to_string()}));
    }
    let tools = registry.list_tools(None);
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
    let views: Vec<Value> = tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            })
        })
        .collect();
    Ok(json!({"names": names, "removed": 0, "tools": views}))
}

struct Silent;

impl ToolHandler for Silent {
    fn call<'call>(
        &'call self,
        _arguments: &'call Object,
        _context: &'call OpenApiCallContext,
    ) -> Pin<Box<dyn Future<Output = Result<OpenApiToolOutput, ToolHandlerError>> + Send + 'call>>
    {
        Box::pin(async { Ok(OpenApiToolOutput::Text(String::new())) })
    }
}

struct LenLoader;

impl ToolHandlerLoader for LenLoader {
    fn load(
        &self,
        handler: &str,
        _config_file_path: Option<&Path>,
    ) -> Result<Option<Arc<dyn ToolHandler>>, HandlerLoadError> {
        if handler == "builtins.len" {
            return Ok(Some(Arc::new(Silent)));
        }
        Ok(None)
    }
}
