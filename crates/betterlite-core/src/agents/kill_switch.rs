use super::json::AgentJson;
use super::validation;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ApiKeyHeader(pub String);
impl Default for ApiKeyHeader {
    fn default() -> Self {
        Self(String::from("x-api-key"))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentKillSwitchAuth {
    Bearer {
        #[serde(skip_serializing)]
        token: SecretString,
    },
    ApiKey {
        #[serde(default)]
        header_name: ApiKeyHeader,
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
        let Some(status_code) = self.status_code else {
            return false;
        };
        (200..300).contains(&status_code)
    }
}
