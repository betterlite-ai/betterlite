use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DailyActivityTable {
    #[serde(rename = "litellm_dailyuserspend")]
    User,
    #[serde(rename = "litellm_dailyteamspend")]
    Team,
    #[serde(rename = "litellm_dailytagspend")]
    Tag,
    #[serde(rename = "litellm_dailyorganizationspend")]
    Organization,
    #[serde(rename = "litellm_dailyenduserspend")]
    Customer,
    #[serde(rename = "litellm_dailyagentspend")]
    Agent,
}

impl DailyActivityTable {
    #[must_use]
    pub const fn metadata(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::User => (
                "litellm_dailyuserspend",
                "user_id",
                "LiteLLM_DailyUserSpend",
            ),
            Self::Team => (
                "litellm_dailyteamspend",
                "team_id",
                "LiteLLM_DailyTeamSpend",
            ),
            Self::Tag => ("litellm_dailytagspend", "tag", "LiteLLM_DailyTagSpend"),
            Self::Organization => (
                "litellm_dailyorganizationspend",
                "organization_id",
                "LiteLLM_DailyOrganizationSpend",
            ),
            Self::Customer => (
                "litellm_dailyenduserspend",
                "end_user_id",
                "LiteLLM_DailyEndUserSpend",
            ),
            Self::Agent => (
                "litellm_dailyagentspend",
                "agent_id",
                "LiteLLM_DailyAgentSpend",
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyActivityScope {
    pub table: DailyActivityTable,
    pub entity_id_field: String,
    pub entity_ids: Option<Vec<String>>,
    pub exclude_entity_ids: Vec<String>,
    pub api_keys: Option<Vec<String>>,
    pub start_date: String,
    pub end_date: String,
    pub model: Option<String>,
    pub timezone_offset_minutes: Option<i32>,
    #[serde(default)]
    pub include_current_utc_day: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SqlQuery {
    pub sql: String,
    pub params: Vec<SqlParameter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SqlParameter {
    Text(String),
    Texts(Vec<String>),
    Integer(i64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportCursor {
    pub date: String,
    pub entity_id: String,
    pub group_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeySpendRow {
    pub api_key: String,
    pub spend: Decimal,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub api_requests: i64,
    pub successful_requests: i64,
    pub failed_requests: i64,
    pub cache_read_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyPage {
    pub rows: Vec<KeySpendRow>,
    pub total_api_keys: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyMetadataRow {
    pub api_key: String,
    pub key_alias: Option<String>,
    pub team_id: Option<String>,
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub key_exists: bool,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportType {
    Daily,
    DailyWithKeys,
    DailyWithModels,
    DailyWithUsers,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportRow {
    pub date: String,
    pub entity_id: String,
    pub entity_alias: Option<String>,
    pub api_key: Option<String>,
    pub key_alias: Option<String>,
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub model: Option<String>,
    pub spend: Decimal,
    pub flat_cost: Decimal,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub api_requests: i64,
    pub successful_requests: i64,
    pub failed_requests: i64,
    pub cache_read_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollupMetricsRow {
    pub date: Option<String>,
    pub api_key: Option<String>,
    pub spend: Option<Decimal>,
    #[serde(default)]
    pub ptu_flat_cost: Option<Decimal>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub cache_creation_input_tokens: Option<i64>,
    pub compression_saved_tokens: Option<i64>,
    pub compression_savings_spend: Option<Decimal>,
    pub prompt_caching_savings_spend: Option<Decimal>,
    #[serde(rename = "gateway_injected_caching_savings_spend")]
    pub gateway_cache_savings_spend: Option<Decimal>,
    pub autorouter_savings_spend: Option<Decimal>,
    pub api_requests: Option<i64>,
    pub successful_requests: Option<i64>,
    pub failed_requests: Option<i64>,
    pub total_response_time_ms: Option<i64>,
    pub timed_requests: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupingSetsRow {
    #[serde(flatten)]
    pub metrics: RollupMetricsRow,
    pub model: Option<String>,
    pub model_group: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub mcp_namespaced_tool_name: Option<String>,
    pub endpoint: Option<String>,
    pub group_level: i64,
    pub distinct_api_keys: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRollupRow {
    #[serde(flatten)]
    pub metrics: RollupMetricsRow,
    pub entity_id: Option<String>,
    pub api_key_rolled: i64,
    pub distinct_api_keys: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatedRows {
    pub grouping_rows: Vec<GroupingSetsRow>,
    pub entity_rows: Option<Vec<EntityRollupRow>>,
    pub distinct_api_keys: i64,
}

pub type SpendLogsWindow = (String, String);
pub type RecoveryFuture<'window> =
    Pin<Box<dyn Future<Output = BTreeMap<String, KeyMetadataRow>> + Send + 'window>>;

pub trait DailyActivityProxyReads {
    fn recover_key_metadata<'window>(
        &'window self,
        resolved: BTreeMap<String, KeyMetadataRow>,
        api_keys: &'window BTreeSet<String>,
        window: Option<SpendLogsWindow>,
    ) -> RecoveryFuture<'window>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyActivityRow {
    pub id: String,
    pub date: String,
    pub api_key: String,
    pub model: Option<String>,
    pub model_group: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub mcp_namespaced_tool_name: Option<String>,
    pub endpoint: Option<String>,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub compression_saved_tokens: i64,
    pub compression_savings_spend: Decimal,
    pub prompt_caching_savings_spend: Decimal,
    #[serde(rename = "gateway_injected_caching_savings_spend")]
    pub gateway_cache_savings_spend: Decimal,
    pub autorouter_savings_spend: Decimal,
    pub spend: Decimal,
    pub api_requests: i64,
    pub successful_requests: i64,
    pub failed_requests: i64,
    pub total_response_time_ms: i64,
    pub timed_requests: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyRowsPage {
    pub total_count: i64,
    pub rows: Vec<DailyActivityRow>,
}

pub fn validate_scope(scope: DailyActivityScope) -> Result<DailyActivityScope, String> {
    if scope.entity_id_field != scope.table.metadata().1 {
        return Err(format!(
            "Invalid entity_id_field {:?} for {}",
            scope.entity_id_field,
            scope.table.metadata().0
        ));
    }
    Ok(scope)
}

mod tests;
