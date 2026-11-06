use std::collections::{BTreeMap, VecDeque};

use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    pub(crate) template: Option<String>,
    pub(crate) timeout: Option<i64>,
    pub(crate) allow_internet_access: Option<bool>,
    pub(crate) metadata: Option<BTreeMap<String, String>>,
    pub(crate) env_vars: Option<BTreeMap<String, String>>,
    pub(crate) resource_limits: Option<BTreeMap<String, String>>,
    pub(crate) resource_requests: Option<BTreeMap<String, String>>,
    pub(crate) entrypoint: Option<Vec<String>>,
    pub(crate) network_policy: Option<Box<RawValue>>,
    pub(crate) secure_access: Option<bool>,
    pub(crate) sandbox_id: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) headers: Option<BTreeMap<String, String>>,
    pub(crate) use_server_proxy: Option<bool>,
    pub(crate) ready_timeout: Option<f64>,
    pub(crate) poll_interval: Option<f64>,
    #[serde(default)]
    pub(crate) responses: Vec<ScriptedResponse>,
    pub(crate) response_body: Option<String>,
    pub(crate) url: Option<String>,
    pub(crate) body: Option<Body>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    CreateBody,
    WaitUntilRunning,
    WaitForExecdEndpoint,
    Run,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScriptedResponse {
    #[serde(default = "default_status")]
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Body {
    pub(crate) code: String,
    pub(crate) context: Context,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Context {
    pub(crate) language: String,
}

impl Input {
    pub(crate) fn scripted(&self) -> VecDeque<ScriptedResponse> {
        if self.responses.is_empty() {
            return VecDeque::from([ScriptedResponse {
                status: 200,
                body: self.response_body.clone().unwrap_or_default(),
            }]);
        }
        VecDeque::from(self.responses.clone())
    }
}

const fn default_status() -> u16 {
    200
}
