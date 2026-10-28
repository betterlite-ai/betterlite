mod auth_wire;
pub mod records;
mod tests;

use auth_wire::ExposedSecret;
use records::AgentKeySummary;

use chrono::{DateTime, FixedOffset, NaiveDateTime};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::net::Ipv6Addr;
use thiserror::Error;
use unicode_normalization::UnicodeNormalization as _;

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
        #[serde(
            serialize_with = "serde_with::As::<serde_with::FromIntoRef<ExposedSecret>>::serialize"
        )]
        token: SecretString,
    },
    ApiKey {
        #[serde(default)]
        header_name: ApiKeyHeader,
        #[serde(
            serialize_with = "serde_with::As::<serde_with::FromIntoRef<ExposedSecret>>::serialize"
        )]
        api_key: SecretString,
    },
    Basic {
        username: String,
        #[serde(
            serialize_with = "serde_with::As::<serde_with::FromIntoRef<ExposedSecret>>::serialize"
        )]
        password: SecretString,
    },
}

impl AgentKillSwitchAuth {
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
    Post,
    Put,
    Patch,
    Delete,
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
        let candidate = value
            .trim_start_matches(|character| character <= ' ')
            .replace(['\t', '\r', '\n'], "");
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
    let unseparated = authority.replace(['@', ':', '#', '?'], "");
    if authority.is_empty()
        || unseparated
            .nfkc()
            .any(|character| "/?#@:".contains(character))
    {
        return false;
    }
    if !authority.contains(['[', ']']) {
        return true;
    }
    if !authority.contains('[') || !authority.contains(']') {
        return false;
    }
    let host_port = authority.rsplit('@').next().unwrap_or_default();
    let host = if let Some(bracketed) = host_port.strip_prefix('[') {
        let Some((literal, suffix)) = bracketed.split_once(']') else {
            return false;
        };
        if !suffix.is_empty() && !suffix.starts_with(':') {
            return false;
        }
        literal
    } else {
        host_port.split(':').next().unwrap_or_default()
    };
    valid_ip_literal(host)
}

fn valid_ip_literal(host: &str) -> bool {
    if let Some(future) = host.strip_prefix(['v', 'V']) {
        return future.split_once('.').is_some_and(|(version, address)| {
            !version.is_empty()
                && version.bytes().all(|byte| byte.is_ascii_hexdigit())
                && !address.is_empty()
        });
    }
    let address = if let Some((address, scope)) = host.split_once('%') {
        if scope.is_empty() || scope.contains('%') {
            return false;
        }
        address
    } else {
        host
    };
    address.parse::<Ipv6Addr>().is_ok()
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
