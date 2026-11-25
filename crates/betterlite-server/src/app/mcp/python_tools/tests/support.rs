use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use betterlite_config::sections::mcp_tools::{McpToolEntry, decode_mcp_tools};
use betterlite_mcp::openapi_tools::output::OpenApiToolOutput;
use betterlite_mcp::openapi_tools::request_headers::OpenApiCallContext;
use betterlite_mcp::tool_registry::handler::ToolHandlerError;
use betterlite_mcp::tool_registry::registry::ToolRegistry;
use betterlite_prompts::arguments::json::{Json, Object};
use pyo3::Python;
use pyo3::types::PyAnyMethods as _;
use tempfile::TempDir;
use tokio::task::yield_now;

use crate::app::mcp::python_tools::{PythonToolsError, load_configured_tools};
use crate::python_hooks::PythonHooks;

pub(super) struct Workspace {
    directory: TempDir,
    pub(super) config_path: PathBuf,
}

impl Workspace {
    pub(super) fn module(&self, name: &str, source: &str) -> String {
        let path = self.directory.path().join(format!("{name}.py"));
        fs::write(&path, source).unwrap();
        name.to_owned()
    }

    pub(super) fn nested_module(&self, folder: &str, name: &str, source: &str) -> String {
        let directory = self.directory.path().join(folder);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(format!("{name}.py")), source).unwrap();
        format!("{folder}.{name}")
    }

    pub(super) fn directory(&self) -> &Path {
        self.directory.path()
    }
}

pub(super) fn workspace() -> Workspace {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("config.yaml");
    fs::write(&config_path, b"model_list: []\n").unwrap();
    Workspace {
        directory,
        config_path,
    }
}

pub(super) struct Fixture {
    pub(super) workspace: Workspace,
    pub(super) hooks: PythonHooks,
    pub(super) registry: ToolRegistry,
}

pub(super) fn fixture() -> Fixture {
    let workspace = workspace();
    let hooks = PythonHooks::new(Some(workspace.config_path.clone()));
    Fixture {
        workspace,
        hooks,
        registry: ToolRegistry::default(),
    }
}

impl Fixture {
    pub(super) async fn load(
        &self,
        entries: Option<&[McpToolEntry]>,
    ) -> Result<(), PythonToolsError> {
        load_configured_tools(
            &self.registry,
            &self.hooks,
            entries,
            Some(&self.workspace.config_path),
        )
        .await
    }

    pub(super) fn names(&self) -> Vec<String> {
        self.registry
            .list_tools(None)
            .into_iter()
            .map(|tool| tool.name)
            .collect()
    }
}

pub(super) fn entries(yaml: &str) -> Vec<McpToolEntry> {
    decode_mcp_tools(yaml.as_bytes()).unwrap().entries
}

pub(super) fn tools_yaml(tools: &[(&str, &str)]) -> String {
    let mut configured = String::from("mcp_tools:\n");
    for &(name, handler) in tools {
        configured.push_str("  - name: ");
        configured.push_str(name);
        configured.push_str("\n    description: runs ");
        configured.push_str(name);
        configured.push_str("\n    handler: ");
        configured.push_str(handler);
        configured.push('\n');
    }
    configured
}

pub(super) async fn load_tool(fixture: &Fixture, module: &str, source: &str, attribute: &str) {
    let qualified = fixture.workspace.module(module, source);
    let handler = format!("{qualified}.{attribute}");
    let configured = entries(&tools_yaml(&[("tool", handler.as_str())]));
    fixture.load(Some(&configured)).await.unwrap();
}

pub(super) async fn call(
    fixture: &Fixture,
    name: &str,
    arguments: &str,
) -> Result<OpenApiToolOutput, ToolHandlerError> {
    let tool = fixture.registry.get_tool(name).unwrap();
    let decoded: Object = serde_json::from_str(arguments).unwrap();
    tool.handler.call(&decoded, &context()).await
}

pub(super) fn context() -> OpenApiCallContext {
    OpenApiCallContext {
        auth_header: Some("Bearer b8871-secret".to_owned()),
        extra_headers: BTreeMap::from([("x-b8871".to_owned(), "extra".to_owned())]),
        resolved_auth_headers: BTreeMap::new(),
    }
}

pub(super) fn prompt_json(json: &str) -> Json {
    serde_json::from_str(json).unwrap()
}

pub(super) async fn yield_times(times: usize) {
    for _ in 0..times {
        yield_now().await;
    }
}

pub(super) fn on_sys_path(directory: &Path) -> bool {
    let listing = directory.to_str().unwrap();
    Python::attach(|py| {
        let path = py.import("sys").unwrap().getattr("path").unwrap();
        path.call_method1("count", (listing,))
            .unwrap()
            .extract::<usize>()
            .unwrap()
            > 0
    })
}
