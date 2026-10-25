use super::json::AgentJson;
use super::validation;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn default_api_key_header() -> String {
    String::from("x-api-key")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentKillSwitchAuth {
    Bearer {
        #[serde(skip_serializing)]
        token: SecretString,
    },
    ApiKey {
        #[serde(default = "default_api_key_header")]
        header_name: String,
        #[serde(skip_serializing)]
        api_key: SecretString,
    },
    Basic {
        username: String,
        #[serde(skip_serializing)]
        password: SecretString,
    },
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum AgentKillSwitchMethod {
    #[default]
    Post,
    Put,
    Patch,
    Delete,
    Get,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentKillSwitchConfig {
    #[serde(deserialize_with = "validation::absolute_http_url")]
    pub url: String,
    #[serde(default)]
    pub method: AgentKillSwitchMethod,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub query_params: BTreeMap<String, String>,
    pub body: Option<BTreeMap<String, AgentJson>>,
    pub auth: Option<AgentKillSwitchAuth>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentKillSwitchResult {
    pub agent_id: String,
    pub url: String,
    pub method: AgentKillSwitchMethod,
    pub status_code: Option<u16>,
    pub response_body: Option<String>,
    pub error: Option<String>,
}
impl AgentKillSwitchResult {
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.status_code
            .is_some_and(|value| (200..300).contains(&value))
    }
}
