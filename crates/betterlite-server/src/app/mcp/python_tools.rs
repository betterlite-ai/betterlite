mod arguments;
mod results;
mod tests;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::slice::from_ref;
use std::sync::Arc;

use betterlite_config::sections::mcp_tools::{McpToolEntry, McpToolSettings};
use betterlite_mcp::openapi_tools::output::OpenApiToolOutput;
use betterlite_mcp::openapi_tools::request_headers::OpenApiCallContext;
use betterlite_mcp::tool_registry::handler::{ToolHandler, ToolHandlerError};
use betterlite_mcp::tool_registry::loader::{HandlerLoadError, ToolHandlerLoader};
use betterlite_mcp::tool_registry::registry::{ToolRegistry, ToolRegistryError};
use betterlite_prompts::arguments::json::Object;
use betterlite_pyhost::blocking_work::WorkError;
use betterlite_pyhost::call_error::CallError;
use pyo3::types::PyAnyMethods as _;
use thiserror::Error;

use crate::python_hooks::HooksError;
use crate::python_hooks::PythonHooks;
use crate::python_hooks::deadline::LOAD_DEADLINE;
use crate::python_hooks::slots::{HookSlot, LoadedHook};

use self::arguments::keyword_arguments;
use self::results::tool_output;

#[derive(Debug, Error)]
pub enum PythonToolsError {
    #[error("could not load the mcp_tools handler {handler:?}: {source}")]
    Load {
        handler: String,
        #[source]
        source: Box<HooksError>,
    },
    #[error(transparent)]
    Registry(#[from] ToolRegistryError),
}

pub async fn load_configured_tools(
    registry: &ToolRegistry,
    hooks: &PythonHooks,
    entries: Option<&[McpToolEntry]>,
    config_file_path: Option<&Path>,
) -> Result<(), PythonToolsError> {
    let Some(configured) = entries else {
        let loader = ReadyLoader { handler: None };
        return registry
            .load_tools_from_config(None, config_file_path, &loader)
            .map_err(PythonToolsError::Registry);
    };
    for entry in configured {
        let loader = ready_loader(hooks, entry).await?;
        registry
            .load_tools_from_config(Some(from_ref(entry)), config_file_path, &loader)
            .map_err(PythonToolsError::Registry)?;
    }
    Ok(())
}

// B4680 registers one entry per call, so the loader only hands back the handle imported just before it
struct ReadyLoader {
    handler: Option<Arc<dyn ToolHandler>>,
}

impl ToolHandlerLoader for ReadyLoader {
    fn load(
        &self,
        _handler: &str,
        _config_file_path: Option<&Path>,
    ) -> Result<Option<Arc<dyn ToolHandler>>, HandlerLoadError> {
        Ok(self.handler.as_ref().map(Arc::clone))
    }
}

async fn ready_loader(
    hooks: &PythonHooks,
    entry: &McpToolEntry,
) -> Result<ReadyLoader, PythonToolsError> {
    let McpToolEntry::Tool(settings) = entry else {
        return Ok(ReadyLoader { handler: None });
    };
    let Some(handler) = configured_handler(settings) else {
        return Ok(ReadyLoader { handler: None });
    };
    let hook = hooks
        .load_hook(HookSlot::McpToolHandler, handler)
        .await
        .map_err(|source| PythonToolsError::Load {
            handler: handler.to_owned(),
            source: Box::new(source),
        })?;
    // get_instance_fn returns the attribute as it is, so a None one is the handler the registry skips
    let tracker = Arc::clone(&hook.tracker);
    let host = hook.host.clone();
    let (skipping, checked) = tracker
        .run(&host, LOAD_DEADLINE, move |py| {
            let skipping = hook.object.loaded.object.bind(py).is_none();
            (skipping, hook)
        })
        .await
        .map_err(|source| PythonToolsError::Load {
            handler: handler.to_owned(),
            source: Box::new(HooksError::Call(CallError::Work(source))),
        })?;
    if skipping {
        return Ok(ReadyLoader { handler: None });
    }
    Ok(ReadyLoader {
        handler: Some(Arc::new(PythonToolHandler { hook: checked })),
    })
}

// LiteLLM's all([name, description, handler]): a missing, null or empty string skips the entry
fn configured_handler(settings: &McpToolSettings) -> Option<&str> {
    settings.name.as_deref().filter(|name| !name.is_empty())?;
    settings
        .description
        .as_deref()
        .filter(|text| !text.is_empty())?;
    settings
        .handler
        .as_deref()
        .filter(|handler| !handler.is_empty())
}

pub struct PythonToolHandler {
    hook: LoadedHook,
}

impl ToolHandler for PythonToolHandler {
    fn call<'call>(
        &'call self,
        arguments: &'call Object,
        _context: &'call OpenApiCallContext,
    ) -> Pin<Box<dyn Future<Output = Result<OpenApiToolOutput, ToolHandlerError>> + Send + 'call>>
    {
        Box::pin(async move {
            let value = self
                .hook
                .call(Vec::new(), keyword_arguments(arguments))
                .await
                .map_err(handler_failure)?;
            Ok(tool_output(value))
        })
    }
}

fn handler_failure(source: CallError) -> ToolHandlerError {
    match source {
        CallError::Python(error) => ToolHandlerError::Failed(error.message),
        CallError::TimedOut { after } | CallError::Work(WorkError::TimedOut { after }) => {
            ToolHandlerError::Failed(format!("the tool handler did not finish within {after:?}"))
        }
        CallError::Value(error) => ToolHandlerError::Failed(error.to_string()),
        CallError::Work(
            work @ (WorkError::ShuttingDown
            | WorkError::Closed { .. }
            | WorkError::Panicked(_)
            | WorkError::Cancelled),
        ) => ToolHandlerError::Failed(work.to_string()),
    }
}
