use betterlite_mcp::client::tools::calling::CallToolOptions;
use betterlite_mcp::tools::tool_calls::CallRequest;
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorKind {
    Plain,
    Broken,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HostKind {
    None,
    Record,
    Fail,
}

fn default_server_url() -> String {
    "https://mcp.example.com/mcp".to_owned()
}

const fn default_timeout_ms() -> u64 {
    60_000
}

const fn default_listing_ms() -> u64 {
    30_000
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Settings {
    #[serde(default = "default_server_url")]
    pub server_url: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_listing_ms")]
    pub listing_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ToolSpec {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Frame {
    #[serde(default)]
    pub delay_ms: u64,
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ListScenario {
    #[serde(default)]
    pub pages: Vec<Frame>,
    #[serde(default)]
    pub error: Option<ErrorKind>,
    #[serde(default)]
    pub cancel: bool,
    #[serde(default)]
    pub raise_on_error: bool,
}

impl CallScenario {
    pub(crate) fn request(&self) -> CallRequest {
        CallRequest {
            name: self.tool.clone(),
            arguments: self.arguments.clone(),
        }
    }

    pub(crate) const fn options(&self) -> CallToolOptions {
        CallToolOptions {
            raise_on_error: self.raise_on_error,
            allow_input_required: matches!(self.input_required, InputRequirement::Accept),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ProgressSpec {
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub total: Option<f64>,
}

fn default_tool() -> String {
    "echo".to_owned()
}

fn default_result_text() -> String {
    "tool output".to_owned()
}

fn default_request_state() -> String {
    "state-1".to_owned()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InputRequirement {
    #[default]
    None,
    Accept,
    Reject,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct CallScenario {
    #[serde(default = "default_tool")]
    pub tool: String,
    #[serde(default)]
    pub arguments: Option<Object>,
    #[serde(default = "default_result_text")]
    pub result_text: String,
    #[serde(default)]
    pub input_required: InputRequirement,
    #[serde(default = "default_request_state")]
    pub request_state: String,
    #[serde(default)]
    pub error: Option<ErrorKind>,
    #[serde(default)]
    pub progress: Vec<ProgressSpec>,
    #[serde(default = "default_host")]
    pub host: HostKind,
    #[serde(default)]
    pub cancel: bool,
    #[serde(default)]
    pub raise_on_error: bool,
}

const fn default_host() -> HostKind {
    HostKind::None
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    ListTools {
        settings: Settings,
        #[serde(default)]
        scenarios: Vec<ListScenario>,
    },
    CallTool {
        settings: Settings,
        #[serde(default)]
        scenarios: Vec<CallScenario>,
    },
}
