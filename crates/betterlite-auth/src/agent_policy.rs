pub mod records;
mod tests;

use records::AgentKeySummary;

use chrono::{DateTime, FixedOffset, NaiveDateTime};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use thiserror::Error;

pub const CALLER_USER_ID_HEADER: &str = "x-litellm-user-id";
pub const CALLER_TEAM_ID_HEADER: &str = "x-litellm-team-id";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct ApiKeyHeader(String);

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

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentKillSwitchAuthView {
    Bearer {
        token: String,
    },
    ApiKey {
        header_name: ApiKeyHeader,
        api_key: String,
    },
    Basic {
        username: String,
        password: String,
    },
}

impl AgentKillSwitchAuth {
    #[must_use]
    pub fn exposed(&self) -> AgentKillSwitchAuthView {
        match self {
            Self::Bearer { token } => AgentKillSwitchAuthView::Bearer {
                token: token.expose_secret().to_owned(),
            },
            Self::ApiKey {
                header_name,
                api_key,
            } => AgentKillSwitchAuthView::ApiKey {
                header_name: header_name.clone(),
                api_key: api_key.expose_secret().to_owned(),
            },
            Self::Basic { username, password } => AgentKillSwitchAuthView::Basic {
                username: username.clone(),
                password: password.expose_secret().to_owned(),
            },
        }
    }

    #[must_use]
    pub fn secret_value(&self) -> &str {
        match self {
            Self::Bearer { token } => token.expose_secret(),
            Self::ApiKey { api_key, .. } => api_key.expose_secret(),
            Self::Basic { password, .. } => password.expose_secret(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum AgentKillSwitchMethod {
    #[default]
    #[serde(rename = "POST")]
    Post,
    #[serde(rename = "PUT")]
    Put,
    #[serde(rename = "PATCH")]
    Patch,
    #[serde(rename = "DELETE")]
    Delete,
    #[serde(rename = "GET")]
    Get,
}

fn default_required_scopes() -> Vec<String> {
    let scope = String::from("user_impersonation");
    vec![scope]
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(try_from = "String")]
pub struct KillSwitchUrl(String);

#[derive(Debug, Error)]
#[error("kill_switch.url must be an absolute http(s) URL")]
pub struct KillSwitchUrlError;

impl TryFrom<String> for KillSwitchUrl {
    type Error = KillSwitchUrlError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let candidate = value.trim_start_matches(|character| character <= ' ');
        let Some((scheme, remainder)) = candidate.split_once(':') else {
            return Err(KillSwitchUrlError);
        };
        let Some(authority) = remainder.strip_prefix("//") else {
            return Err(KillSwitchUrlError);
        };
        if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https")
            || !has_valid_authority(authority)
        {
            return Err(KillSwitchUrlError);
        }
        Ok(Self(value))
    }
}

fn has_valid_authority(value: &str) -> bool {
    let authority = value.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return false;
    }
    if authority.starts_with('[') {
        return authority.find(']').is_some_and(|end| {
            let Some(after_bracket) = end.checked_add(1).and_then(|index| authority.get(index..))
            else {
                return false;
            };
            !after_bracket.contains('[') && !after_bracket.contains(']')
        });
    }
    !authority.contains(['[', ']'])
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentKillSwitchConfig {
    pub url: KillSwitchUrl,
    #[serde(default)]
    pub method: AgentKillSwitchMethod,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub query_params: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<BTreeMap<String, Box<RawValue>>>,
    #[serde(default)]
    pub auth: Option<AgentKillSwitchAuth>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentExecutionMode {
    #[default]
    Autonomous,
    Delegated,
    Both,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentIdentityBinding {
    pub agent_id: String,
    #[serde(default)]
    pub active: PolicyFlag,
    pub provider: IdentityProvider,
    pub tenant_id: String,
    pub client_id: String,
    #[serde(default)]
    pub service_principal_id: Option<String>,
    pub issuer: String,
    #[serde(default)]
    pub required_roles: Vec<String>,
    #[serde(default = "default_required_scopes")]
    pub required_scopes: Vec<String>,
    pub revision: String,
    #[serde(default)]
    pub last_authenticated_at: Option<PolicyDateTime>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityProvider {
    MicrosoftEntra,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum PolicyDateTime {
    Offset(DateTime<FixedOffset>),
    Naive(NaiveDateTime),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ManagedAgentContext {
    pub agent_id: String,
    #[serde(default)]
    pub binding_revision: Option<String>,
    pub mode: ManagedAgentMode,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub subject_oid: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedAgentMode {
    Autonomous,
    Delegated,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentResponse {
    #[serde(default)]
    pub identity: Option<AgentIdentityBinding>,
    #[serde(default)]
    pub identity_managed: bool,
    #[serde(default)]
    pub enabled: PolicyFlag,
    #[serde(default)]
    pub execution_mode: AgentExecutionMode,
    #[serde(default)]
    pub jwt_auth_configured: bool,
    pub agent_id: String,
    pub agent_name: String,
    #[serde(default)]
    pub litellm_params: Option<BTreeMap<String, Box<RawValue>>>,
    pub agent_card_params: BTreeMap<String, Box<RawValue>>,
    #[serde(default)]
    pub object_permission: Option<BTreeMap<String, Box<RawValue>>>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub spend: Option<Decimal>,
    #[serde(default)]
    pub tpm_limit: Option<i64>,
    #[serde(default)]
    pub rpm_limit: Option<i64>,
    #[serde(default)]
    pub session_tpm_limit: Option<i64>,
    #[serde(default)]
    pub session_rpm_limit: Option<i64>,
    #[serde(default)]
    pub static_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub extra_headers: Option<Vec<String>>,
    #[serde(default)]
    pub access_group_ids: Option<Vec<String>>,
    #[serde(default)]
    pub kill_switch: Option<AgentKillSwitchConfig>,
    #[serde(default)]
    pub keys: Option<Vec<AgentKeySummary>>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub search_score: Option<Decimal>,
    #[serde(default)]
    pub created_at: Option<PolicyDateTime>,
    #[serde(default)]
    pub updated_at: Option<PolicyDateTime>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub updated_by: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct PolicyFlag(pub bool);

impl Default for PolicyFlag {
    fn default() -> Self {
        Self(true)
    }
}
