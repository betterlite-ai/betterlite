use betterlite_auth::identity;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_with::rust::double_option;
use std::collections::BTreeMap;

pub mod card;
pub mod json;
pub mod kill_switch;
pub mod permissions;
pub mod public;
mod tests;
mod validation;

pub type AgentCapabilities = card::AgentCapabilities;
pub type AgentCard = card::AgentCard;
pub type AgentCardSignature = card::AgentCardSignature;
pub type AgentExtension = card::AgentExtension;
pub type AgentInterface = card::AgentInterface;
pub type AgentProvider = card::AgentProvider;
pub type AgentSkill = card::AgentSkill;
pub type AugmentedAgentCard = card::AugmentedAgentCard;
pub type OAuthFlows = card::OAuthFlows;
pub type SecurityApiKey = card::SecurityApiKey;
pub type SecurityHttp = card::SecurityHttp;
pub type SecurityLocation = card::SecurityLocation;
pub type SecurityMutualTls = card::SecurityMutualTls;
pub type SecurityOauth2 = card::SecurityOauth2;
pub type SecurityOpenIdConnect = card::SecurityOpenIdConnect;
pub type SecurityScheme = card::SecurityScheme;
use json::AgentJson;
pub type AgentObjectPermission = permissions::AgentObjectPermission;
pub type AgentKillSwitchAuth = kill_switch::AgentKillSwitchAuth;
pub type AgentKillSwitchConfig = kill_switch::AgentKillSwitchConfig;
pub type AgentKillSwitchMethod = kill_switch::AgentKillSwitchMethod;
pub type AgentKillSwitchResult = kill_switch::AgentKillSwitchResult;

pub const CALLER_USER_ID_HEADER: &str = "x-litellm-user-id";
pub const CALLER_TEAM_ID_HEADER: &str = "x-litellm-team-id";

macro_rules! record {
    ($name:ident { $($fields:tt)* }) => {
        #[derive(Clone, Debug, Deserialize, Serialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name { $($fields)* }
    };
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct AgentConfig {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub identity: Option<Option<identity::EntraIdentityConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "validation::non_null_option")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "validation::non_null_option")]
    pub execution_mode: Option<identity::ExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "validation::non_null_option")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "validation::non_null_option")]
    pub agent_card_params: Option<AgentCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "validation::non_null_option")]
    pub litellm_params: Option<BTreeMap<String, AgentJson>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "validation::non_null_option")]
    pub object_permission: Option<AgentObjectPermission>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub tpm_limit: Option<Option<i64>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub rpm_limit: Option<Option<i64>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub session_tpm_limit: Option<Option<i64>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub session_rpm_limit: Option<Option<i64>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub static_headers: Option<Option<BTreeMap<String, String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub extra_headers: Option<Option<Vec<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub access_group_ids: Option<Option<Vec<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub kill_switch: Option<Option<AgentKillSwitchConfig>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct PatchAgentRequest {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub identity: Option<Option<identity::EntraIdentityConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "validation::non_null_option")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "validation::non_null_option")]
    pub execution_mode: Option<identity::ExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "validation::non_null_option")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "validation::non_null_option")]
    pub agent_card_params: Option<AgentCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "validation::non_null_option")]
    pub litellm_params: Option<BTreeMap<String, AgentJson>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "validation::non_null_option")]
    pub object_permission: Option<AgentObjectPermission>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub tpm_limit: Option<Option<i64>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub rpm_limit: Option<Option<i64>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub session_tpm_limit: Option<Option<i64>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub session_rpm_limit: Option<Option<i64>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub static_headers: Option<Option<BTreeMap<String, String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub extra_headers: Option<Option<Vec<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub access_group_ids: Option<Option<Vec<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub kill_switch: Option<Option<AgentKillSwitchConfig>>,
}

record! { AgentCaller {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
} }
record! { AgentKeySummary {
    pub token: String,
    pub key_alias: Option<String>,
    pub key_name: Option<String>,
} }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentResponse {
    pub identity: Option<identity::AgentIdentityBinding>,
    #[serde(default)]
    pub identity_managed: bool,
    #[serde(default)]
    pub enabled: identity::ActiveState,
    #[serde(default)]
    pub execution_mode: identity::ExecutionMode,
    #[serde(default)]
    pub jwt_auth_configured: bool,
    pub agent_id: String,
    pub agent_name: String,
    pub litellm_params: Option<BTreeMap<String, AgentJson>>,
    pub agent_card_params: AgentCard,
    pub object_permission: Option<AgentObjectPermission>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub spend: Option<Decimal>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub session_tpm_limit: Option<i64>,
    pub session_rpm_limit: Option<i64>,
    pub static_headers: Option<BTreeMap<String, String>>,
    pub extra_headers: Option<Vec<String>>,
    pub access_group_ids: Option<Vec<String>>,
    pub kill_switch: Option<AgentKillSwitchConfig>,
    pub keys: Option<Vec<AgentKeySummary>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub search_score: Option<Decimal>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
}

pub type AgentMakePublicResponse = public::AgentMakePublicResponse;
pub type MakeAgentsPublicRequest = public::MakeAgentsPublicRequest;
