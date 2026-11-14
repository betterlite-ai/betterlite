use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Registry(RegistryInput),
    Create(CreateInput),
    CreateAndRun(RunInput),
    Delete(DeleteInput),
}

#[derive(Debug, Deserialize)]
pub(crate) struct RegistryInput {
    pub(crate) tools: Vec<ToolInput>,
    pub(crate) names: Vec<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolInput {
    pub(crate) sandbox_tool_name: Option<String>,
    pub(crate) litellm_params: Option<ParamsInput>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ParamsInput {
    pub(crate) sandbox_provider: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateInput {
    pub(crate) provider: String,
    pub(crate) template: Option<String>,
    pub(crate) timeout: Option<i64>,
    pub(crate) allow_internet_access: Option<bool>,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) metadata: Option<BTreeMap<String, String>>,
    pub(crate) env_vars: Option<BTreeMap<String, String>>,
    pub(crate) resource_limits: Option<BTreeMap<String, String>>,
    pub(crate) resource_requests: Option<BTreeMap<String, String>>,
    pub(crate) entrypoint: Option<Vec<String>>,
    pub(crate) network_policy: Option<serde_json::Value>,
    pub(crate) secure_access: Option<bool>,
    pub(crate) use_server_proxy: Option<bool>,
    pub(crate) ready_timeout: Option<f64>,
    pub(crate) poll_interval: Option<f64>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) replies: Vec<Reply>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RunInput {
    #[serde(flatten)]
    pub(crate) create: CreateInput,
    pub(crate) code: String,
    pub(crate) language: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeleteInput {
    pub(crate) provider: String,
    pub(crate) container: String,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) replies: Vec<Reply>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Reply {
    pub(crate) status: Option<u16>,
    pub(crate) body: Option<String>,
}
