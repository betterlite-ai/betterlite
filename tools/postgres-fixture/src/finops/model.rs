use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use subtle::ConstantTimeEq as _;

pub(super) fn secret_eq(left: &SecretString, right: &SecretString) -> bool {
    bool::from(
        left.expose_secret()
            .as_bytes()
            .ct_eq(right.expose_secret().as_bytes()),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganizationRow {
    pub organization_id: String,
    pub organization_alias: String,
    pub budget_id: String,
    pub created_by: String,
    pub updated_by: String,
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyMetadataRow {
    pub token: SecretString,
    #[serde(default)]
    pub key_alias: Option<String>,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub team_organization_id: Option<String>,
}

impl PartialEq for KeyMetadataRow {
    fn eq(&self, other: &Self) -> bool {
        secret_eq(&self.token, &other.token)
            && self.key_alias == other.key_alias
            && self.organization_id == other.organization_id
            && self.team_organization_id == other.team_organization_id
    }
}

impl Eq for KeyMetadataRow {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigName {
    VantageSettings,
    CloudzeroSettings,
}

impl ConfigName {
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::VantageSettings => "vantage_settings",
            Self::CloudzeroSettings => "cloudzero_settings",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigRow {
    pub name: ConfigName,
    #[serde(default)]
    pub serialized_value: Option<SecretString>,
}

impl PartialEq for ConfigRow {
    fn eq(&self, other: &Self) -> bool {
        let value_matches = match (&self.serialized_value, &other.serialized_value) {
            (Some(left), Some(right)) => secret_eq(left, right),
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        };
        self.name == other.name && value_matches
    }
}

impl Eq for ConfigRow {}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageApiKey {
    Key(SecretString),
    OrphanHash(String),
}

impl PartialEq for UsageApiKey {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Key(left), Self::Key(right)) => secret_eq(left, right),
            (Self::OrphanHash(left), Self::OrphanHash(right)) => left == right,
            (Self::Key(_), Self::OrphanHash(_)) | (Self::OrphanHash(_), Self::Key(_)) => false,
        }
    }
}

impl Eq for UsageApiKey {}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinOpsUsageRow {
    pub id: String,
    #[serde(default)]
    pub user_id: Option<String>,
    pub date: String,
    pub api_key: UsageApiKey,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_group: Option<String>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub mcp_namespaced_tool_name: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub cache_read_input_tokens: i64,
    #[serde(default)]
    pub cache_creation_input_tokens: i64,
    #[serde(default)]
    pub compression_saved_tokens: i64,
    #[serde(default)]
    pub compression_savings_spend: Decimal,
    #[serde(default)]
    pub prompt_caching_savings_spend: Decimal,
    // the pinned column name outruns the name-length lint, so the Rust field is shorter
    #[serde(default, rename = "gateway_injected_caching_savings_spend")]
    pub gateway_injected_savings: Decimal,
    #[serde(default)]
    pub autorouter_savings_spend: Decimal,
    #[serde(default)]
    pub spend: Decimal,
    #[serde(default)]
    pub api_requests: i64,
    #[serde(default)]
    pub successful_requests: i64,
    #[serde(default)]
    pub failed_requests: i64,
    #[serde(default)]
    pub total_response_time_ms: i64,
    #[serde(default)]
    pub timed_requests: i64,
    pub created_at: String,
    pub updated_at: String,
}
