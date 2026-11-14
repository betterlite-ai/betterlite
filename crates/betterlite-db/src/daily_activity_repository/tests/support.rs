#![cfg(test)]

use super::super::DailyActivityRepository;
use super::super::database::{DailyActivityDatabase, DailyRowFilter, DatabaseFuture};
use super::super::rows::records::{DeletedVerificationTokenRow, VerificationTokenRow};
use super::super::rows::{DatabaseRow, DatabaseValue, RowError};
use crate::budget_permission_rows::JsonData;
use crate::daily_activity_contracts::identifiers::DailyActivityTable;
use crate::daily_activity_contracts::{
    DailyActivityProxyReads, DailyActivityScope, DailyActivityScopeInput, KeyMetadataRow,
    SpendLogsWindow, SqlQuery,
};
use crate::postgres_store::errors::DbError;
use chrono::{DateTime, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::ready;
use std::sync::{Arc, Mutex};
use tokio::sync::Barrier;

pub(crate) type FakeResult<T> = std::result::Result<T, FakeFailure>;
type CountReply<'database> = DatabaseFuture<'database, i64>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FakeFailure {
    Unavailable,
    InvalidRow,
}

fn fake_error(failure: FakeFailure) -> DbError<RowError> {
    match failure {
        FakeFailure::Unavailable => DbError::NoDbConnected,
        FakeFailure::InvalidRow => DbError::InvalidRecord {
            source: RowError::MissingColumn {
                column: "fault".to_owned(),
            },
        },
    }
}

#[derive(Clone)]
pub(crate) enum RawReply {
    Rows(Vec<DatabaseRow>),
    None,
    Failure(FakeFailure),
    Pending,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DailyCall {
    pub(crate) table: DailyActivityTable,
    pub(crate) filters: Vec<DailyRowFilter>,
    pub(crate) skip: i64,
    pub(crate) take: i64,
}

pub(crate) struct FakeDatabase {
    pub(crate) raw: Mutex<VecDeque<RawReply>>,
    pub(crate) queries: Mutex<Vec<SqlQuery>>,
    pub(crate) active: Mutex<FakeResult<Vec<VerificationTokenRow>>>,
    pub(crate) active_calls: Mutex<usize>,
    pub(crate) deleted: Mutex<FakeResult<Vec<DeletedVerificationTokenRow>>>,
    pub(crate) deleted_pending: bool,
    pub(crate) deleted_calls: Mutex<Vec<Vec<String>>>,
    pub(crate) count: Mutex<FakeResult<i64>>,
    pub(crate) rows: Mutex<FakeResult<Vec<crate::daily_activity_contracts::DailyActivityRow>>>,
    pub(crate) daily_calls: Mutex<Vec<DailyCall>>,
    pub(crate) daily_barrier: Option<Arc<Barrier>>,
    pub(crate) aggregate_barrier: Option<Arc<Barrier>>,
}

impl Default for FakeDatabase {
    fn default() -> Self {
        Self {
            raw: Mutex::new(VecDeque::new()),
            queries: Mutex::new(Vec::new()),
            active: Mutex::new(Ok(Vec::new())),
            active_calls: Mutex::new(0),
            deleted: Mutex::new(Ok(Vec::new())),
            deleted_pending: false,
            deleted_calls: Mutex::new(Vec::new()),
            count: Mutex::new(Ok(0)),
            rows: Mutex::new(Ok(Vec::new())),
            daily_calls: Mutex::new(Vec::new()),
            daily_barrier: None,
            aggregate_barrier: None,
        }
    }
}

impl FakeDatabase {
    pub(crate) fn push_raw(&self, reply: RawReply) {
        self.raw.lock().unwrap().push_back(reply);
    }

    pub(crate) fn set_raw_rows(&self, rows: Vec<DatabaseRow>) {
        self.push_raw(RawReply::Rows(rows));
    }

    pub(crate) fn query_count(&self) -> usize {
        self.queries.lock().unwrap().len()
    }

    pub(crate) fn last_query(&self) -> SqlQuery {
        self.queries.lock().unwrap().last().cloned().unwrap()
    }

    pub(crate) fn active_call_count(&self) -> usize {
        *self.active_calls.lock().unwrap()
    }

    pub(crate) fn deleted_call_args(&self) -> Vec<Vec<String>> {
        self.deleted_calls.lock().unwrap().clone()
    }

    pub(crate) fn daily_call_count(&self) -> usize {
        self.daily_calls.lock().unwrap().len()
    }

    fn record_daily(
        &self,
        table: DailyActivityTable,
        filters: &[DailyRowFilter],
        skip: i64,
        take: i64,
    ) {
        self.daily_calls.lock().unwrap().push(DailyCall {
            table,
            filters: filters.to_vec(),
            skip,
            take,
        });
    }
}

impl DailyActivityDatabase for FakeDatabase {
    fn query_raw<'database>(
        &'database self,
        query: &'database SqlQuery,
    ) -> DatabaseFuture<'database, Option<Vec<DatabaseRow>>> {
        self.queries.lock().unwrap().push(query.clone());
        let barrier = self.aggregate_barrier.clone();
        let reply = self
            .raw
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(RawReply::Rows(Vec::new()));
        Box::pin(async move {
            if let Some(barrier) = barrier {
                barrier.wait().await;
            }
            match reply {
                RawReply::Rows(rows) => Ok(Some(rows)),
                RawReply::None => Ok(None),
                RawReply::Failure(failure) => Err(fake_error(failure)),
                RawReply::Pending => {
                    std::future::pending::<()>().await;
                    Ok(None)
                }
            }
        })
    }

    fn active_tokens<'database>(
        &'database self,
        _tokens: &'database [String],
    ) -> DatabaseFuture<'database, Vec<VerificationTokenRow>> {
        *self.active_calls.lock().unwrap() += 1;
        reply_future(self.active.lock().unwrap().clone())
    }

    fn deleted_tokens<'database>(
        &'database self,
        tokens: &'database [String],
    ) -> DatabaseFuture<'database, Vec<DeletedVerificationTokenRow>> {
        self.deleted_calls.lock().unwrap().push(tokens.to_vec());
        let pending = self.deleted_pending;
        let reply = self.deleted.lock().unwrap().clone();
        Box::pin(async move {
            if pending {
                std::future::pending::<()>().await;
            }
            match reply {
                Ok(rows) => Ok(rows),
                Err(failure) => Err(fake_error(failure)),
            }
        })
    }

    fn count_daily<'database>(
        &'database self,
        table: DailyActivityTable,
        filters: &'database [DailyRowFilter],
    ) -> CountReply<'database> {
        self.record_daily(table, filters, 0, 0);
        let reply = self.count.lock().unwrap().clone();
        barrier_reply(self.daily_barrier.clone(), reply)
    }

    fn find_daily<'database>(
        &'database self,
        table: DailyActivityTable,
        filters: &'database [DailyRowFilter],
        skip: i64,
        take: i64,
    ) -> DatabaseFuture<'database, Vec<crate::daily_activity_contracts::DailyActivityRow>> {
        self.record_daily(table, filters, skip, take);
        barrier_reply(
            self.daily_barrier.clone(),
            self.rows.lock().unwrap().clone(),
        )
    }
}

fn reply_future<T>(reply: FakeResult<T>) -> DatabaseFuture<'static, T>
where
    T: Send + 'static,
{
    barrier_reply(None, reply)
}

fn barrier_reply<T>(
    barrier: Option<Arc<Barrier>>,
    reply: FakeResult<T>,
) -> DatabaseFuture<'static, T>
where
    T: Send + 'static,
{
    Box::pin(async move {
        if let Some(barrier) = barrier {
            barrier.wait().await;
        }
        match reply {
            Ok(value) => Ok(value),
            Err(failure) => Err(fake_error(failure)),
        }
    })
}

#[derive(Clone)]
pub(crate) struct RecoveryCall {
    pub(crate) resolved: BTreeMap<String, KeyMetadataRow>,
    pub(crate) api_keys: BTreeSet<String>,
    pub(crate) window: Option<SpendLogsWindow>,
}

pub(crate) struct FakeReads {
    pub(crate) calls: Mutex<Vec<RecoveryCall>>,
    pub(crate) reply: Mutex<BTreeMap<String, KeyMetadataRow>>,
}

impl Default for FakeReads {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            reply: Mutex::new(BTreeMap::new()),
        }
    }
}

impl FakeReads {
    pub(crate) fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub(crate) fn last_call(&self) -> RecoveryCall {
        self.calls.lock().unwrap().last().cloned().unwrap()
    }
}

impl DailyActivityProxyReads for FakeReads {
    fn recover_key_metadata(
        &self,
        resolved: BTreeMap<String, KeyMetadataRow>,
        api_keys: &BTreeSet<String>,
        window: Option<SpendLogsWindow>,
    ) -> impl Future<Output = BTreeMap<String, KeyMetadataRow>> + Send {
        self.calls.lock().unwrap().push(RecoveryCall {
            resolved,
            api_keys: api_keys.clone(),
            window,
        });
        let reply = self.reply.lock().unwrap().clone();
        ready(reply)
    }
}

pub(crate) fn fixed_now() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(1_786_000_000, 0).unwrap()
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

pub(crate) fn scope(table: DailyActivityTable) -> DailyActivityScope {
    let input = DailyActivityScopeInput {
        table,
        entity_id_field: table.metadata().entity_field.to_owned(),
        entity_ids: None,
        exclude_entity_ids: Vec::new(),
        api_keys: None,
        start_date: "2026-08-01".to_owned(),
        end_date: "2026-08-05".to_owned(),
        model: None,
        timezone_offset_minutes: None,
        include_current_utc_day: false,
    };
    DailyActivityScope::try_from(input).unwrap()
}

pub(crate) fn text_row(values: Vec<(&str, DatabaseValue)>) -> DatabaseRow {
    DatabaseRow {
        values: values
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    }
}

pub(crate) fn optional_text(value: Option<&str>) -> DatabaseValue {
    value.map_or(DatabaseValue::Null, |text| {
        DatabaseValue::Text(text.to_owned())
    })
}

pub(crate) fn money(units: i64, scale: u32) -> DatabaseValue {
    DatabaseValue::Number(Decimal::new(units, scale))
}

pub(crate) fn count(value: i64) -> DatabaseValue {
    DatabaseValue::Integer(value)
}

fn base_values() -> Vec<(&'static str, DatabaseValue)> {
    vec![
        ("date", DatabaseValue::Text("2026-08-01".to_owned())),
        ("entity_id", DatabaseValue::Text("entity".to_owned())),
        ("entity_alias", optional_text(None)),
        ("api_key", optional_text(None)),
        ("key_alias", optional_text(None)),
        ("user_id", optional_text(Some("user"))),
        ("user_email", optional_text(None)),
        ("model", optional_text(Some("gpt"))),
        ("model_group", optional_text(None)),
        ("custom_llm_provider", optional_text(None)),
        ("mcp_namespaced_tool_name", optional_text(None)),
        ("endpoint", optional_text(None)),
        ("spend", money(125, 2)),
        ("flat_cost", money(0, 2)),
        ("ptu_flat_cost", money(0, 2)),
        ("prompt_tokens", count(1)),
        ("completion_tokens", count(2)),
        ("total_tokens", count(3)),
        ("api_requests", count(4)),
        ("successful_requests", count(5)),
        ("failed_requests", count(6)),
        ("cache_read_input_tokens", count(7)),
        ("cache_creation_input_tokens", count(8)),
        ("compression_saved_tokens", count(0)),
        ("compression_savings_spend", money(0, 2)),
        ("prompt_caching_savings_spend", money(0, 2)),
        ("gateway_injected_caching_savings_spend", money(0, 2)),
        ("autorouter_savings_spend", money(0, 2)),
        ("total_response_time_ms", count(9)),
        ("timed_requests", count(10)),
        ("group_level", count(1)),
        ("api_key_rolled", count(0)),
        ("distinct_api_keys", DatabaseValue::Null),
    ]
}

pub(crate) fn key_spend_row(api_key: &str) -> DatabaseRow {
    let mut values = base_values();
    values.push(("api_key", DatabaseValue::Text(api_key.to_owned())));
    text_row(values)
}

pub(crate) fn export_row(index: usize) -> DatabaseRow {
    let mut values = base_values();
    values.push(("date", DatabaseValue::Text(format!("2026-08-{index:02}"))));
    values.push(("api_key", DatabaseValue::Text("key".to_owned())));
    text_row(values)
}

pub(crate) fn token_row(token: &str) -> VerificationTokenRow {
    VerificationTokenRow {
        token: token.to_owned(),
        key_alias: None,
        team_id: None,
        user_id: None,
        metadata: None,
    }
}

pub(crate) fn tags_metadata(tags: &[&str]) -> BTreeMap<String, JsonData> {
    BTreeMap::from([(
        "tags".to_owned(),
        JsonData::Array(
            tags.iter()
                .map(|tag| JsonData::String((*tag).to_owned()))
                .collect(),
        ),
    )])
}

pub(crate) fn deleted_row(
    token: &str,
    deleted_at: NaiveDateTime,
    key_alias: Option<&str>,
) -> DeletedVerificationTokenRow {
    DeletedVerificationTokenRow {
        row: VerificationTokenRow {
            token: token.to_owned(),
            key_alias: key_alias.map(str::to_owned),
            team_id: None,
            user_id: None,
            metadata: None,
        },
        deleted_at,
    }
}

pub(crate) fn moment(seconds: i64) -> NaiveDateTime {
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .unwrap()
        .naive_utc()
}

pub(crate) fn grouping_row(distinct_api_keys: Option<i64>) -> DatabaseRow {
    let mut values = base_values();
    values.push((
        "distinct_api_keys",
        distinct_api_keys.map_or(DatabaseValue::Null, count),
    ));
    text_row(values)
}

pub(crate) fn entity_row(entity_id: Option<&str>, api_key_rolled: i64) -> DatabaseRow {
    let mut values = base_values();
    values.push(("entity_id", optional_text(entity_id)));
    values.push(("api_key_rolled", count(api_key_rolled)));
    text_row(values)
}

pub(crate) fn without(mut row: DatabaseRow, column: &str) -> DatabaseRow {
    row.values.remove(column);
    row
}

pub(crate) fn with_column(mut row: DatabaseRow, column: &str, value: DatabaseValue) -> DatabaseRow {
    row.values.insert(column.to_owned(), value);
    row
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }
}

impl std::io::Write for CapturedLogs {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (CapturedLogs, impl tracing::Subscriber + Send + Sync) {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(logs.clone())
        .finish();
    (logs, subscriber)
}
