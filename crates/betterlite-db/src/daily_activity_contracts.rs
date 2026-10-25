use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::time::SystemTime;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
pub mod identifiers;
pub type DailyActivityError = identifiers::DailyActivityError;
pub type DailyActivityMetadata = identifiers::DailyActivityMetadata;
pub type DailyActivityTable = identifiers::DailyActivityTable;
pub type EntityIdField = identifiers::EntityIdField;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DailyActivityScopeInput {
    pub table: DailyActivityTable,
    pub entity_id_field: EntityIdField,
    #[serde(deserialize_with = "Option::deserialize")]
    pub entity_ids: Option<Vec<String>>,
    pub exclude_entity_ids: Vec<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub api_keys: Option<Vec<String>>,
    pub start_date: String,
    pub end_date: String,
    #[serde(deserialize_with = "Option::deserialize")]
    pub model: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub timezone_offset_minutes: Option<i32>,
    #[serde(default)]
    pub include_current_utc_day: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DailyActivityScopeInput")]
pub struct DailyActivityScope {
    table: DailyActivityTable,
    entity_id_field: EntityIdField,
    #[serde(deserialize_with = "Option::deserialize")]
    entity_ids: Option<Vec<String>>,
    exclude_entity_ids: Vec<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    api_keys: Option<Vec<String>>,
    start_date: String,
    end_date: String,
    #[serde(deserialize_with = "Option::deserialize")]
    model: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    timezone_offset_minutes: Option<i32>,
    #[serde(default)]
    include_current_utc_day: bool,
}

impl TryFrom<DailyActivityScopeInput> for DailyActivityScope {
    type Error = DailyActivityError;

    fn try_from(input: DailyActivityScopeInput) -> Result<Self, Self::Error> {
        if input.entity_id_field != input.table.metadata().entity_field {
            return Err(DailyActivityError::EntityField {
                field: input.entity_id_field.wire_name().to_owned(),
                table: input.table,
            });
        }
        Ok(Self {
            table: input.table,
            entity_id_field: input.entity_id_field,
            entity_ids: input.entity_ids,
            exclude_entity_ids: input.exclude_entity_ids,
            api_keys: input.api_keys,
            start_date: input.start_date,
            end_date: input.end_date,
            model: input.model,
            timezone_offset_minutes: input.timezone_offset_minutes,
            include_current_utc_day: input.include_current_utc_day,
        })
    }
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
    #[serde(deserialize_with = "Option::deserialize")]
    pub key_alias: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub team_id: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub user_id: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
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
    #[serde(deserialize_with = "Option::deserialize")]
    pub entity_alias: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub api_key: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub key_alias: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub user_id: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub user_email: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
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
    #[serde(deserialize_with = "Option::deserialize")]
    pub date: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub api_key: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub spend: Option<Decimal>,
    #[serde(default)]
    pub ptu_flat_cost: Option<Decimal>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub prompt_tokens: Option<i64>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub completion_tokens: Option<i64>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub cache_read_input_tokens: Option<i64>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub cache_creation_input_tokens: Option<i64>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub compression_saved_tokens: Option<i64>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub compression_savings_spend: Option<Decimal>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub prompt_caching_savings_spend: Option<Decimal>,
    #[serde(rename = "gateway_injected_caching_savings_spend")]
    #[serde(deserialize_with = "Option::deserialize")]
    pub gateway_cache_savings_spend: Option<Decimal>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub autorouter_savings_spend: Option<Decimal>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub api_requests: Option<i64>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub successful_requests: Option<i64>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub failed_requests: Option<i64>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub total_response_time_ms: Option<i64>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub timed_requests: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupingSetsRow {
    #[serde(flatten)]
    pub metrics: RollupMetricsRow,
    #[serde(deserialize_with = "Option::deserialize")]
    pub model: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub model_group: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub custom_llm_provider: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub mcp_namespaced_tool_name: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub endpoint: Option<String>,
    pub group_level: i64,
    #[serde(deserialize_with = "Option::deserialize")]
    pub distinct_api_keys: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRollupRow {
    #[serde(flatten)]
    pub metrics: RollupMetricsRow,
    #[serde(deserialize_with = "Option::deserialize")]
    pub entity_id: Option<String>,
    pub api_key_rolled: i64,
    #[serde(deserialize_with = "Option::deserialize")]
    pub distinct_api_keys: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatedRows {
    pub grouping_rows: Vec<GroupingSetsRow>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub entity_rows: Option<Vec<EntityRollupRow>>,
    pub distinct_api_keys: i64,
}

pub type SpendLogsWindow = (SystemTime, SystemTime);
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
    #[serde(deserialize_with = "Option::deserialize")]
    pub model: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub model_group: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub custom_llm_provider: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub mcp_namespaced_tool_name: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
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

mod tests;
