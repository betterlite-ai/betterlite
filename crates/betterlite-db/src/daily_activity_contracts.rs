use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::time::SystemTime;
pub mod identifiers;
use identifiers::{DailyActivityError, DailyActivityTable};
macro_rules! records {
    ($($name:ident { $($kind:ty => $($(#[$attribute:meta])* $field:ident),+);+ $(;)? })*) => {
        $(#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        pub struct $name {
            $($( $(#[$attribute])* #[serde(deserialize_with = "Deserialize::deserialize")]
            pub $field: $kind, )+)+
        })*
    };
}
records! {
    DailyActivityScopeInput {
        DailyActivityTable => table;
        String => entity_id_field, start_date, end_date;
        Option<Vec<String>> => entity_ids, api_keys;
        Vec<String> => exclude_entity_ids;
        Option<String> => model;
        Option<i64> => timezone_offset_minutes;
        bool => #[serde(default)] include_current_utc_day;
    }
    SqlQuery {
        String => sql;
        Vec<SqlParameter> => params;
    }
    ExportCursor {
        String => date, entity_id, group_key;
    }
    KeySpendRow {
        String => api_key;
        Decimal => spend;
        i64 => prompt_tokens, completion_tokens, total_tokens, api_requests, successful_requests, failed_requests, cache_read_input_tokens, cache_creation_input_tokens;
    }
    KeyPage {
        Vec<KeySpendRow> => rows;
        i64 => total_api_keys;
    }
    KeyMetadataRow {
        String => api_key;
        Option<String> => key_alias, team_id, user_id, user_email;
        bool => key_exists;
        Vec<String> => tags;
    }
    ExportRow {
        String => date, entity_id;
        Option<String> => entity_alias, api_key, key_alias, user_id, user_email, model;
        Decimal => spend, flat_cost;
        i64 => prompt_tokens, completion_tokens, api_requests, successful_requests, failed_requests, cache_read_input_tokens, cache_creation_input_tokens;
    }
    RollupMetricsRow {
        Option<String> => date, api_key;
        Option<Decimal> => spend, #[serde(default)] ptu_flat_cost, compression_savings_spend, prompt_caching_savings_spend, #[serde(rename = "gateway_injected_caching_savings_spend")] gateway_cache_savings_spend, autorouter_savings_spend;
        Option<i64> => prompt_tokens, completion_tokens, cache_read_input_tokens, cache_creation_input_tokens, compression_saved_tokens, api_requests, successful_requests, failed_requests, total_response_time_ms, timed_requests;
    }
    GroupingSetsRow {
        RollupMetricsRow => #[serde(flatten)] metrics;
        Option<String> => model, model_group, custom_llm_provider, mcp_namespaced_tool_name, endpoint;
        i64 => group_level;
        Option<i64> => distinct_api_keys;
    }
    EntityRollupRow {
        RollupMetricsRow => #[serde(flatten)] metrics;
        Option<String> => entity_id;
        i64 => api_key_rolled;
        Option<i64> => distinct_api_keys;
    }
    AggregatedRows {
        Vec<GroupingSetsRow> => grouping_rows;
        Option<Vec<EntityRollupRow>> => entity_rows;
        i64 => distinct_api_keys;
    }
    DailyActivityRow {
        String => id, date, api_key;
        Option<String> => model, model_group, custom_llm_provider, mcp_namespaced_tool_name, endpoint;
        i64 => prompt_tokens, completion_tokens, cache_read_input_tokens, cache_creation_input_tokens, compression_saved_tokens, api_requests, successful_requests, failed_requests, total_response_time_ms, timed_requests;
        Decimal => compression_savings_spend, prompt_caching_savings_spend, #[serde(rename = "gateway_injected_caching_savings_spend")] gateway_cache_savings_spend, autorouter_savings_spend, spend;
    }
    DailyRowsPage {
        i64 => total_count;
        Vec<DailyActivityRow> => rows;
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DailyActivityScopeInput")]
pub struct DailyActivityScope {
    #[serde(flatten)]
    input: DailyActivityScopeInput,
}
impl TryFrom<DailyActivityScopeInput> for DailyActivityScope {
    type Error = DailyActivityError;
    fn try_from(input: DailyActivityScopeInput) -> Result<Self, Self::Error> {
        if input.entity_id_field != input.table.metadata().entity_field {
            return Err(DailyActivityError::EntityField {
                field: input.entity_id_field,
                table: input.table.metadata().table_name,
            });
        }
        Ok(Self { input })
    }
}
impl AsRef<DailyActivityScopeInput> for DailyActivityScope {
    fn as_ref(&self) -> &DailyActivityScopeInput {
        &self.input
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SqlParameter {
    Text(String),
    Texts(Vec<String>),
    Integer(i64),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportType {
    Daily,
    DailyWithKeys,
    DailyWithModels,
    DailyWithUsers,
}
pub type SpendLogsWindow = (SystemTime, SystemTime);
pub trait DailyActivityProxyReads {
    fn recover_key_metadata(
        &self,
        resolved: BTreeMap<String, KeyMetadataRow>,
        api_keys: &BTreeSet<String>,
        window: Option<SpendLogsWindow>,
    ) -> impl Future<Output = BTreeMap<String, KeyMetadataRow>> + Send;
}
mod tests;
