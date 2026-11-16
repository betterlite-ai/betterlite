#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::SystemTime;

use betterlite_analytics::daily_metrics::{
    BreakdownMetrics, DailySpendData, KeyMetadata, SpendMetrics,
};
use betterlite_db::daily_activity_contracts::identifiers::DailyActivityTable;
use betterlite_db::daily_activity_contracts::{
    AggregatedRows, DailyActivityScope, DailyActivityScopeInput, EntityRollupRow, GroupingSetsRow,
    KeyMetadataRow, RollupMetricsRow, SpendLogsWindow,
};
use betterlite_db::daily_activity_repository::DailyActivityRepositoryError;
use betterlite_db::daily_activity_sql::DailyActivitySqlError;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;

use super::AggregatedActivityRepository;

pub(crate) const DAY_ONE: &str = "2024-03-05";
pub(crate) const DAY_TWO: &str = "2024-03-06";
pub(crate) const KEY_ONE: &str = "key-one";
pub(crate) const KEY_TWO: &str = "key-two";
pub(crate) const SENTINEL: &str = "__ptu_flat_cost__";

pub(crate) const fn blank_metrics() -> RollupMetricsRow {
    RollupMetricsRow {
        date: None,
        api_key: None,
        spend: None,
        ptu_flat_cost: None,
        compression_savings_spend: None,
        prompt_caching_savings_spend: None,
        gateway_cache_savings_spend: None,
        autorouter_savings_spend: None,
        prompt_tokens: None,
        completion_tokens: None,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        compression_saved_tokens: None,
        api_requests: None,
        successful_requests: None,
        failed_requests: None,
        total_response_time_ms: None,
        timed_requests: None,
    }
}

pub(crate) struct RowParts<'row> {
    pub(crate) level: i64,
    pub(crate) date: Option<&'row str>,
    pub(crate) api_key: Option<&'row str>,
    pub(crate) model: Option<&'row str>,
    pub(crate) model_group: Option<&'row str>,
    pub(crate) provider: Option<&'row str>,
    pub(crate) mcp: Option<&'row str>,
    pub(crate) endpoint: Option<&'row str>,
    pub(crate) spend: Option<i64>,
    pub(crate) flat_cost: Option<i64>,
    pub(crate) prompt_tokens: Option<i64>,
    pub(crate) completion_tokens: Option<i64>,
}

impl<'row> RowParts<'row> {
    pub(crate) const fn new(level: i64, date: Option<&'row str>) -> Self {
        Self {
            level,
            date,
            api_key: None,
            model: None,
            model_group: None,
            provider: None,
            mcp: None,
            endpoint: None,
            spend: None,
            flat_cost: None,
            prompt_tokens: None,
            completion_tokens: None,
        }
    }
}

pub(crate) fn row(parts: RowParts<'_>) -> GroupingSetsRow {
    GroupingSetsRow {
        metrics: RollupMetricsRow {
            date: parts.date.map(str::to_owned),
            api_key: parts.api_key.map(str::to_owned),
            spend: parts.spend.map(amount),
            ptu_flat_cost: parts.flat_cost.map(amount),
            prompt_tokens: parts.prompt_tokens,
            completion_tokens: parts.completion_tokens,
            ..blank_metrics()
        },
        model: parts.model.map(str::to_owned),
        model_group: parts.model_group.map(str::to_owned),
        custom_llm_provider: parts.provider.map(str::to_owned),
        mcp_namespaced_tool_name: parts.mcp.map(str::to_owned),
        endpoint: parts.endpoint.map(str::to_owned),
        group_level: parts.level,
        distinct_api_keys: None,
    }
}

pub(crate) struct EntityParts<'row> {
    pub(crate) date: Option<&'row str>,
    pub(crate) entity_id: Option<&'row str>,
    pub(crate) api_key: Option<&'row str>,
    pub(crate) rolled: bool,
    pub(crate) distinct_api_keys: Option<i64>,
    pub(crate) spend: Option<i64>,
}

impl<'row> EntityParts<'row> {
    pub(crate) const fn new(date: Option<&'row str>, entity_id: Option<&'row str>) -> Self {
        Self {
            date,
            entity_id,
            api_key: None,
            rolled: false,
            distinct_api_keys: None,
            spend: None,
        }
    }
}

pub(crate) fn entity(parts: EntityParts<'_>) -> EntityRollupRow {
    EntityRollupRow {
        metrics: RollupMetricsRow {
            date: parts.date.map(str::to_owned),
            api_key: parts.api_key.map(str::to_owned),
            spend: parts.spend.map(amount),
            ..blank_metrics()
        },
        entity_id: parts.entity_id.map(str::to_owned),
        api_key_rolled: i64::from(parts.rolled),
        distinct_api_keys: parts.distinct_api_keys,
    }
}

pub(crate) fn amount(value: i64) -> Decimal {
    Decimal::from(value)
}

pub(crate) fn zero_metrics() -> SpendMetrics {
    SpendMetrics::default()
}

pub(crate) const fn empty_aggregated() -> AggregatedRows {
    AggregatedRows {
        grouping_rows: Vec::new(),
        entity_rows: None,
        distinct_api_keys: 0,
    }
}

pub(crate) fn day(date: &str) -> DailySpendData {
    DailySpendData {
        date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
        metrics: SpendMetrics::default(),
        breakdown: BreakdownMetrics::default(),
    }
}

pub(crate) fn key_row(api_key: &str, key_exists: bool) -> KeyMetadataRow {
    KeyMetadataRow {
        api_key: api_key.to_owned(),
        key_alias: Some(format!("alias-{api_key}")),
        team_id: Some("team-alpha".to_owned()),
        user_id: Some("user-alpha".to_owned()),
        user_email: Some(format!("{api_key}@example.com")),
        key_exists,
        tags: Vec::new(),
    }
}

pub(crate) fn key_map(rows: &[KeyMetadataRow]) -> BTreeMap<String, KeyMetadataRow> {
    let mut map = BTreeMap::new();
    for row in rows {
        map.insert(row.api_key.clone(), row.clone());
    }
    map
}

pub(crate) fn key_metadata_for(api_key: &str, key_exists: bool) -> KeyMetadata {
    KeyMetadata {
        key_alias: Some(format!("alias-{api_key}")),
        team_id: Some("team-alpha".to_owned()),
        user_id: Some("user-alpha".to_owned()),
        user_email: Some(format!("{api_key}@example.com")),
        key_exists: Some(key_exists),
    }
}

pub(crate) const fn missing_key_metadata() -> KeyMetadata {
    KeyMetadata {
        key_alias: None,
        team_id: None,
        user_id: None,
        user_email: None,
        key_exists: Some(false),
    }
}

pub(crate) fn scope() -> DailyActivityScope {
    DailyActivityScope::try_from(DailyActivityScopeInput {
        table: DailyActivityTable::Team,
        entity_id_field: "team_id".to_owned(),
        start_date: "2024-03-01".to_owned(),
        end_date: "2024-03-31".to_owned(),
        entity_ids: None,
        api_keys: None,
        exclude_entity_ids: Vec::new(),
        model: None,
        timezone_offset_minutes: None,
        include_current_utc_day: false,
    })
    .unwrap()
}

pub(crate) const fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
}

pub(crate) fn window(start: DateTime<Utc>, end: DateTime<Utc>) -> SpendLogsWindow {
    (SystemTime::from(start), SystemTime::from(end))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RepositoryCall {
    Aggregated {
        start_date: String,
        end_date: String,
        include_entity_breakdown: bool,
        api_key_limit: i64,
    },
    Metadata {
        api_keys: BTreeSet<String>,
        window: Option<SpendLogsWindow>,
    },
}

pub(crate) struct FakeRepository {
    pub(crate) calls: Mutex<Vec<RepositoryCall>>,
    pub(crate) rows: AggregatedRows,
    pub(crate) aggregate_fails: bool,
    pub(crate) metadata: BTreeMap<String, KeyMetadataRow>,
    pub(crate) metadata_fails: bool,
}

pub(crate) const fn fake(rows: AggregatedRows) -> FakeRepository {
    FakeRepository {
        calls: Mutex::new(Vec::new()),
        rows,
        aggregate_fails: false,
        metadata: BTreeMap::new(),
        metadata_fails: false,
    }
}

pub(crate) const fn fake_failing_aggregate() -> FakeRepository {
    let mut repository = fake(empty_aggregated());
    repository.aggregate_fails = true;
    repository
}

pub(crate) const fn fake_failing_metadata(rows: AggregatedRows) -> FakeRepository {
    let mut repository = fake(rows);
    repository.metadata_fails = true;
    repository
}

pub(crate) fn recorded_calls(repository: &FakeRepository) -> Vec<RepositoryCall> {
    repository.calls.lock().unwrap().clone()
}

impl AggregatedActivityRepository for FakeRepository {
    async fn read_aggregated(
        &self,
        scope: &DailyActivityScope,
        include_entity_breakdown: bool,
        api_key_limit: i64,
    ) -> Result<AggregatedRows, DailyActivityRepositoryError> {
        let input = scope.as_ref();
        self.calls.lock().unwrap().push(RepositoryCall::Aggregated {
            start_date: input.start_date.clone(),
            end_date: input.end_date.clone(),
            include_entity_breakdown,
            api_key_limit,
        });
        if self.aggregate_fails {
            return Err(DailyActivityRepositoryError::from(
                DailyActivitySqlError::Timezone,
            ));
        }
        Ok(self.rows.clone())
    }

    async fn read_key_metadata(
        &self,
        api_keys: &BTreeSet<String>,
        window: Option<SpendLogsWindow>,
    ) -> Result<BTreeMap<String, KeyMetadataRow>, DailyActivityRepositoryError> {
        self.calls.lock().unwrap().push(RepositoryCall::Metadata {
            api_keys: api_keys.clone(),
            window,
        });
        if self.metadata_fails {
            return Err(DailyActivityRepositoryError::from(
                DailyActivitySqlError::Timezone,
            ));
        }
        Ok(self.metadata.clone())
    }
}
