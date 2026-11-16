#![cfg(test)]

use betterlite_db::daily_activity_contracts::identifiers::DailyActivityTable;
use betterlite_db::daily_activity_contracts::{
    DailyActivityProxyReads, DailyActivityRow, KeyMetadataRow, SpendLogsWindow, SqlQuery,
};
use betterlite_db::daily_activity_repository::DailyActivityRepository;
use betterlite_db::daily_activity_repository::database::{DailyActivityDatabase, DatabaseFuture};
use betterlite_db::daily_activity_repository::rows::records::{
    DeletedVerificationTokenRow, VerificationTokenRow,
};
use betterlite_db::daily_activity_repository::rows::{DatabaseRow, RowError};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_models::decisions::Json;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::Mutex;
use std::time::SystemTime;

use betterlite_analytics::daily_metrics::{BreakdownMetrics, SpendMetrics};

use super::breakdown::{BreakdownContext, update_breakdown_metrics};
use super::error::PagesError;
use super::metadata::{EntityMetadata, MetadataFields};
use super::read::{DailyActivityPageRequest, PageCallbacks};
use super::record::DailySpendRecord;

pub(crate) fn text(value: &str) -> String {
    value.to_owned()
}

pub(crate) fn money(value: &str) -> Decimal {
    value.parse::<Decimal>().unwrap()
}

pub(crate) fn metrics(spend: Decimal, total_tokens: i64, api_requests: i64) -> SpendMetrics {
    SpendMetrics {
        spend,
        flat_cost: Decimal::ZERO,
        prompt_tokens: 0,
        completion_tokens: 0,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
        compression_saved_tokens: 0,
        compression_savings_spend: Decimal::ZERO,
        prompt_caching_savings_spend: Decimal::ZERO,
        gateway_caching_savings: Decimal::ZERO,
        autorouter_savings_spend: Decimal::ZERO,
        total_tokens,
        successful_requests: 0,
        failed_requests: 0,
        api_requests,
        total_response_time_ms: 0,
        timed_requests: 0,
    }
}

pub(crate) fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_hms_opt(hour, minute, 0)
        .unwrap()
        .and_utc()
}

pub(crate) fn day(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

pub(crate) fn window(start: DateTime<Utc>, end: DateTime<Utc>) -> SpendLogsWindow {
    (SystemTime::from(start), SystemTime::from(end))
}

pub(crate) fn keys(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn fields(entries: &[(&str, &str)]) -> MetadataFields {
    entries
        .iter()
        .map(|(name, value)| (text(name), Json::Text(text(value))))
        .collect()
}

pub(crate) fn entity_metadata(entries: &[(&str, MetadataFields)]) -> EntityMetadata {
    entries
        .iter()
        .map(|(entity, metadata)| (text(entity), metadata.clone()))
        .collect()
}

pub(crate) fn key_row(api_key: &str, alias: &str) -> KeyMetadataRow {
    KeyMetadataRow {
        api_key: text(api_key),
        key_alias: Some(text(alias)),
        team_id: Some(text("team-1")),
        user_id: Some(text("user-1")),
        user_email: Some(text("user@example.com")),
        key_exists: true,
        tags: Vec::new(),
    }
}

pub(crate) fn spend_record(date: &str, api_key: &str) -> DailySpendRecord {
    DailySpendRecord {
        date: text(date),
        api_key: text(api_key),
        model: None,
        model_group: None,
        custom_llm_provider: None,
        mcp_namespaced_tool_name: None,
        endpoint: None,
        prompt_tokens: 0,
        completion_tokens: 0,
        spend: Decimal::ZERO,
        flat_cost: Decimal::ZERO,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
        compression_saved_tokens: 0,
        compression_savings_spend: Decimal::ZERO,
        prompt_caching_savings_spend: Decimal::ZERO,
        gateway_caching_savings: Decimal::ZERO,
        autorouter_savings_spend: Decimal::ZERO,
        api_requests: 0,
        successful_requests: 0,
        failed_requests: 0,
        total_response_time_ms: 0,
        timed_requests: 0,
        entity_id: None,
    }
}

pub(crate) fn row(id: &str, date: &str, api_key: &str) -> DailyActivityRow {
    DailyActivityRow {
        id: text(id),
        date: text(date),
        api_key: text(api_key),
        model: None,
        model_group: None,
        custom_llm_provider: None,
        mcp_namespaced_tool_name: None,
        endpoint: None,
        prompt_tokens: 0,
        completion_tokens: 0,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
        compression_saved_tokens: 0,
        api_requests: 0,
        successful_requests: 0,
        failed_requests: 0,
        total_response_time_ms: 0,
        timed_requests: 0,
        compression_savings_spend: Decimal::ZERO,
        prompt_caching_savings_spend: Decimal::ZERO,
        gateway_cache_savings_spend: Decimal::ZERO,
        autorouter_savings_spend: Decimal::ZERO,
        spend: Decimal::ZERO,
    }
}

pub(crate) fn request<'text>(
    start_date: &'text str,
    end_date: &'text str,
) -> DailyActivityPageRequest<'text> {
    DailyActivityPageRequest {
        table: DailyActivityTable::User,
        entity_id_field: "user_id",
        entity_id: None,
        exclude_entity_ids: Vec::new(),
        start_date: Some(start_date),
        end_date: Some(end_date),
        model: None,
        api_key: None,
        page: 1,
        page_size: 10,
        timezone_offset_minutes: None,
        include_current_utc_day: false,
        entity_metadata: None,
        callbacks: PageCallbacks::default(),
    }
}

#[derive(Default)]
pub(crate) struct BreakdownFixture {
    pub(crate) model_metadata: BTreeMap<String, MetadataFields>,
    pub(crate) provider_metadata: BTreeMap<String, MetadataFields>,
    pub(crate) api_key_metadata: BTreeMap<String, KeyMetadataRow>,
    pub(crate) entity_id_field: Option<&'static str>,
    pub(crate) entity_metadata: Option<EntityMetadata>,
}

impl BreakdownFixture {
    pub(crate) fn context(&self) -> BreakdownContext<'_> {
        BreakdownContext {
            model_metadata: &self.model_metadata,
            provider_metadata: &self.provider_metadata,
            api_key_metadata: &self.api_key_metadata,
            entity_id_field: self.entity_id_field,
            entity_metadata: self.entity_metadata.as_ref(),
        }
    }

    #[must_use]
    pub(crate) fn with_key_row(mut self, row: KeyMetadataRow) -> Self {
        self.api_key_metadata.insert(row.api_key.clone(), row);
        self
    }

    #[must_use]
    pub(crate) fn with_entity_field(mut self, field: &'static str) -> Self {
        self.entity_id_field = Some(field);
        self
    }

    #[must_use]
    pub(crate) fn with_entity_metadata(mut self, metadata: EntityMetadata) -> Self {
        self.entity_metadata = Some(metadata);
        self
    }
}

pub(crate) fn breakdown_of(
    records: &[DailySpendRecord],
    fixture: &BreakdownFixture,
) -> Result<BreakdownMetrics, PagesError> {
    let mut breakdown = BreakdownMetrics::default();
    for record in records {
        update_breakdown_metrics(&mut breakdown, record, &fixture.context())?;
    }
    Ok(breakdown)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FindCall {
    pub(crate) table: DailyActivityTable,
    pub(crate) predicate: SqlQuery,
    pub(crate) skip: i64,
    pub(crate) take: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReadCall {
    pub(crate) api_keys: BTreeSet<String>,
    pub(crate) window: Option<SpendLogsWindow>,
}

pub(crate) struct FakeDatabase {
    pub(crate) total_count: i64,
    pub(crate) rows: Vec<DailyActivityRow>,
    pub(crate) fail: bool,
    pub(crate) count_calls: Mutex<Vec<DailyActivityTable>>,
    pub(crate) find_calls: Mutex<Vec<FindCall>>,
    pub(crate) token_calls: Mutex<Vec<Vec<String>>>,
}

pub(crate) fn page(rows: Vec<DailyActivityRow>, total_count: i64) -> FakeDatabase {
    FakeDatabase {
        total_count,
        rows,
        fail: false,
        count_calls: Mutex::new(Vec::new()),
        find_calls: Mutex::new(Vec::new()),
        token_calls: Mutex::new(Vec::new()),
    }
}

pub(crate) fn failing() -> FakeDatabase {
    let mut database = page(Vec::new(), 0);
    database.fail = true;
    database
}

impl FakeDatabase {
    pub(crate) fn find_calls(&self) -> Vec<FindCall> {
        self.find_calls.lock().unwrap().clone()
    }

    pub(crate) fn count_calls(&self) -> Vec<DailyActivityTable> {
        self.count_calls.lock().unwrap().clone()
    }

    pub(crate) fn token_calls(&self) -> Vec<Vec<String>> {
        self.token_calls.lock().unwrap().clone()
    }
}

impl DailyActivityDatabase for FakeDatabase {
    fn query_raw<'database>(
        &'database self,
        _query: &'database SqlQuery,
    ) -> DatabaseFuture<'database, Option<Vec<DatabaseRow>>> {
        Box::pin(ready(Ok(None)))
    }

    fn active_tokens<'database>(
        &'database self,
        tokens: &'database [String],
    ) -> DatabaseFuture<'database, Vec<VerificationTokenRow>> {
        self.token_calls.lock().unwrap().push(tokens.to_vec());
        let reply: Result<Vec<VerificationTokenRow>, DbError<RowError>> = if self.fail {
            Err(DbError::NoDbConnected)
        } else {
            Ok(Vec::new())
        };
        Box::pin(ready(reply))
    }

    fn deleted_tokens<'database>(
        &'database self,
        _tokens: &'database [String],
    ) -> DatabaseFuture<'database, Vec<DeletedVerificationTokenRow>> {
        Box::pin(ready(Ok(Vec::new())))
    }

    fn count_daily<'database>(
        &'database self,
        table: DailyActivityTable,
        _predicate: &'database SqlQuery,
    ) -> DatabaseFuture<'database, i64> {
        self.count_calls.lock().unwrap().push(table);
        let reply: Result<i64, DbError<RowError>> = if self.fail {
            Err(DbError::NoDbConnected)
        } else {
            Ok(self.total_count)
        };
        Box::pin(ready(reply))
    }

    fn find_daily<'database>(
        &'database self,
        table: DailyActivityTable,
        predicate: &'database SqlQuery,
        skip: i64,
        take: i64,
    ) -> DatabaseFuture<'database, Vec<DailyActivityRow>> {
        self.find_calls.lock().unwrap().push(FindCall {
            table,
            predicate: predicate.clone(),
            skip,
            take,
        });
        let reply: Result<Vec<DailyActivityRow>, DbError<RowError>> = if self.fail {
            Err(DbError::NoDbConnected)
        } else {
            Ok(self.rows.clone())
        };
        Box::pin(ready(reply))
    }
}

pub(crate) struct FakeReads {
    pub(crate) recovered: BTreeMap<String, KeyMetadataRow>,
    pub(crate) calls: Mutex<Vec<ReadCall>>,
}

pub(crate) fn reads(recovered: BTreeMap<String, KeyMetadataRow>) -> FakeReads {
    FakeReads {
        recovered,
        calls: Mutex::new(Vec::new()),
    }
}

impl FakeReads {
    pub(crate) fn read_calls(&self) -> Vec<ReadCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl DailyActivityProxyReads for FakeReads {
    fn recover_key_metadata(
        &self,
        resolved: BTreeMap<String, KeyMetadataRow>,
        api_keys: &BTreeSet<String>,
        window: Option<SpendLogsWindow>,
    ) -> impl Future<Output = BTreeMap<String, KeyMetadataRow>> + Send {
        self.calls.lock().unwrap().push(ReadCall {
            api_keys: api_keys.clone(),
            window,
        });
        let mut merged = resolved;
        merged.extend(self.recovered.clone());
        ready(merged)
    }
}

pub(crate) fn fixed_now() -> DateTime<Utc> {
    at(2025, 12, 31, 23, 59)
}

pub(crate) fn repository(
    database: FakeDatabase,
    reads: FakeReads,
) -> DailyActivityRepository<FakeDatabase, FakeReads, fn() -> DateTime<Utc>> {
    DailyActivityRepository {
        database,
        proxy_reads: reads,
        clock: fixed_now,
    }
}

pub(crate) fn page_repository(
    rows: Vec<DailyActivityRow>,
    total_count: i64,
) -> DailyActivityRepository<FakeDatabase, FakeReads, fn() -> DateTime<Utc>> {
    repository(page(rows, total_count), reads(BTreeMap::new()))
}

pub(crate) fn failing_repository()
-> DailyActivityRepository<FakeDatabase, FakeReads, fn() -> DateTime<Utc>> {
    repository(failing(), reads(BTreeMap::new()))
}
