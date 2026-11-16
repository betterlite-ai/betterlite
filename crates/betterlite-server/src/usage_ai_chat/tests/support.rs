#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_analytics::daily_metrics::{
    BreakdownMetrics, DailySpendData, DailySpendMetadata, DailySpendPage, MetricWithMetadata,
    SpendMetrics,
};
use betterlite_db::daily_activity_contracts::identifiers::DailyActivityTable;
use betterlite_db::daily_activity_contracts::{
    DailyActivityProxyReads, DailyActivityRow, DailyActivityScope, DailyActivityScopeInput,
    KeyMetadataRow, SpendLogsWindow, SqlQuery,
};
use betterlite_db::daily_activity_repository::DailyActivityRepository;
use betterlite_db::daily_activity_repository::database::{DailyActivityDatabase, DatabaseFuture};
use betterlite_db::daily_activity_repository::rows::DatabaseRow;
use betterlite_db::daily_activity_repository::rows::records::{
    DeletedVerificationTokenRow, VerificationTokenRow,
};
use betterlite_models::decisions::Json;
use chrono::{DateTime, NaiveDate, Utc};
use futures_util::Stream;
use futures_util::stream;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tokio::sync::mpsc;

use crate::daily_activity::pages::read::DailyActivityPageRequest;
use crate::daily_activity::recovery::scope::ScopeIds;
use crate::usage_ai_chat::error::UsageChatError;
use crate::usage_ai_chat::flow::stream_usage_ai_chat;
use crate::usage_ai_chat::messages::ChatMessage;
use crate::usage_ai_chat::model::{ModelReply, ModelRequest, ModelToolCall, UsageChatModel};
use crate::usage_ai_chat::parts::{UsageChatParts, UsageChatRequest};
use crate::usage_ai_chat::source::{
    DailyActivityUsageData, UsageActivityRepository, UsageDataSource,
};
use crate::usage_ai_chat::tools::ToolDefinition;

pub(crate) fn money(text: &str) -> Decimal {
    text.parse().unwrap()
}

pub(crate) fn on(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

pub(crate) fn now() -> DateTime<Utc> {
    on(2026, 11, 16).and_hms_opt(0, 0, 0).unwrap().and_utc()
}

pub(crate) fn metrics(spend: &str, api_requests: i64, total_tokens: i64) -> SpendMetrics {
    SpendMetrics {
        spend: money(spend),
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

pub(crate) fn metadata(
    spend: &str,
    requests: i64,
    successful: i64,
    failed: i64,
    tokens: i64,
) -> DailySpendMetadata {
    DailySpendMetadata {
        total_spend: money(spend),
        total_flat_cost: Decimal::ZERO,
        total_prompt_tokens: 0,
        total_completion_tokens: 0,
        total_tokens: tokens,
        total_api_requests: requests,
        total_successful_requests: successful,
        total_failed_requests: failed,
        total_cache_read_input_tokens: 0,
        total_cache_creation_tokens: 0,
        total_compression_saved_tokens: 0,
        total_compression_savings: Decimal::ZERO,
        total_prompt_caching_savings: Decimal::ZERO,
        total_gateway_caching_savings: Decimal::ZERO,
        total_autorouter_savings_spend: Decimal::ZERO,
        total_response_time_ms: 0,
        total_timed_requests: 0,
        page: 1,
        total_pages: 1,
        has_more: false,
        api_key_limit: None,
        total_api_keys: None,
        entity_total_api_keys: None,
    }
}

pub(crate) fn entry(spend: &str, api_requests: i64, total_tokens: i64) -> MetricWithMetadata {
    MetricWithMetadata {
        metrics: metrics(spend, api_requests, total_tokens),
        metadata: IndexMap::new(),
        api_key_breakdown: IndexMap::new(),
    }
}

pub(crate) fn alias_entry(
    alias: &str,
    spend: &str,
    api_requests: i64,
    total_tokens: i64,
) -> MetricWithMetadata {
    let mut metadata: IndexMap<String, Json> = IndexMap::new();
    metadata.insert("alias".to_owned(), Json::Text(alias.to_owned()));
    MetricWithMetadata {
        metrics: metrics(spend, api_requests, total_tokens),
        metadata,
        api_key_breakdown: IndexMap::new(),
    }
}

pub(crate) fn non_text_alias_entry(
    spend: &str,
    api_requests: i64,
    total_tokens: i64,
) -> MetricWithMetadata {
    let mut metadata: IndexMap<String, Json> = IndexMap::new();
    metadata.insert("alias".to_owned(), Json::Bool(true));
    MetricWithMetadata {
        metrics: metrics(spend, api_requests, total_tokens),
        metadata,
        api_key_breakdown: IndexMap::new(),
    }
}

pub(crate) fn breakdown(
    models: Vec<(&str, MetricWithMetadata)>,
    providers: Vec<(&str, MetricWithMetadata)>,
    entities: Vec<(&str, MetricWithMetadata)>,
) -> BreakdownMetrics {
    let mut value = BreakdownMetrics::default();
    for (name, entry) in models {
        value.models.insert(name.to_owned(), entry);
    }
    for (name, entry) in providers {
        value.providers.insert(name.to_owned(), entry);
    }
    for (name, entry) in entities {
        value.entities.insert(name.to_owned(), entry);
    }
    value
}

pub(crate) fn day(date: NaiveDate, breakdown: BreakdownMetrics) -> DailySpendData {
    DailySpendData {
        date,
        metrics: metrics("0", 0, 0),
        breakdown,
    }
}

pub(crate) fn page(results: Vec<DailySpendData>, metadata: DailySpendMetadata) -> DailySpendPage {
    DailySpendPage { results, metadata }
}

pub(crate) fn empty_page() -> DailySpendPage {
    page(Vec::new(), metadata("0", 0, 0, 0, 0))
}

pub(crate) fn answer(content: &str) -> ModelReply {
    ModelReply {
        content: Some(content.to_owned()),
        tool_calls: Vec::new(),
    }
}

pub(crate) fn tool_call(name: &str, arguments: &str) -> ModelReply {
    ModelReply {
        content: None,
        tool_calls: vec![ModelToolCall {
            id: "call-1".to_owned(),
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        }],
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct FakeFailure(pub(crate) String);

pub(crate) enum ReplyStep {
    Reply(ModelReply),
    Fail(FakeFailure),
}

pub(crate) enum StreamStep {
    Chunks(Vec<String>),
    Fail(FakeFailure),
}

#[derive(Debug, Clone)]
pub(crate) struct RecordedRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<ChatMessage>,
    pub(crate) tools: Option<Vec<ToolDefinition>>,
    pub(crate) temperature: f64,
}

impl RecordedRequest {
    pub(crate) fn capture(request: &ModelRequest<'_>) -> Self {
        Self {
            model: request.model.to_owned(),
            messages: request.messages.to_vec(),
            tools: request.tools.map(|tools| tools.to_vec()),
            temperature: request.temperature,
        }
    }
}

pub(crate) struct FakeModel {
    pub(crate) completions: Mutex<VecDeque<ReplyStep>>,
    pub(crate) streams: Mutex<VecDeque<StreamStep>>,
    pub(crate) requests: Mutex<Vec<RecordedRequest>>,
}

pub(crate) fn fake_model() -> FakeModel {
    FakeModel {
        completions: Mutex::new(VecDeque::new()),
        streams: Mutex::new(VecDeque::new()),
        requests: Mutex::new(Vec::new()),
    }
}

impl FakeModel {
    pub(crate) fn replying(self, reply: ModelReply) -> Self {
        self.completions
            .lock()
            .unwrap()
            .push_back(ReplyStep::Reply(reply));
        self
    }

    pub(crate) fn failing_completion(self, message: &str) -> Self {
        self.completions
            .lock()
            .unwrap()
            .push_back(ReplyStep::Fail(FakeFailure(message.to_owned())));
        self
    }

    pub(crate) fn streaming(self, chunks: &[&str]) -> Self {
        let chunks = chunks
            .iter()
            .map(|chunk| (*chunk).to_owned())
            .collect::<Vec<String>>();
        self.streams
            .lock()
            .unwrap()
            .push_back(StreamStep::Chunks(chunks));
        self
    }

    pub(crate) fn failing_stream(self, message: &str) -> Self {
        self.streams
            .lock()
            .unwrap()
            .push_back(StreamStep::Fail(FakeFailure(message.to_owned())));
        self
    }

    pub(crate) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl UsageChatModel for FakeModel {
    type Error = FakeFailure;

    fn complete(
        &self,
        request: &ModelRequest<'_>,
    ) -> impl Future<Output = Result<ModelReply, FakeFailure>> + Send {
        let recorded = RecordedRequest::capture(request);
        let step = self
            .completions
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted completion");
        async move {
            self.requests.lock().unwrap().push(recorded);
            match step {
                ReplyStep::Reply(reply) => Ok(reply),
                ReplyStep::Fail(error) => Err(error),
            }
        }
    }

    fn stream(
        &self,
        request: &ModelRequest<'_>,
    ) -> impl Future<
        Output = Result<impl Stream<Item = Result<String, FakeFailure>> + Send, FakeFailure>,
    > + Send {
        let recorded = RecordedRequest::capture(request);
        let step = self
            .streams
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted stream");
        async move {
            self.requests.lock().unwrap().push(recorded);
            match step {
                StreamStep::Chunks(chunks) => {
                    let items: Vec<Result<String, FakeFailure>> =
                        chunks.into_iter().map(Ok).collect();
                    Ok(stream::iter(items))
                }
                StreamStep::Fail(error) => Err(error),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceCall {
    Global {
        start_date: String,
        end_date: String,
        user_id: Option<String>,
    },
    Teams {
        start_date: String,
        end_date: String,
        team_ids: Option<Vec<String>>,
    },
    Tags {
        start_date: String,
        end_date: String,
        tags: Option<Vec<String>>,
    },
}

pub(crate) struct FakeSource {
    pub(crate) calls: Mutex<Vec<SourceCall>>,
    pub(crate) pages: Mutex<VecDeque<Result<DailySpendPage, UsageChatError>>>,
}

pub(crate) fn fake_source() -> FakeSource {
    FakeSource {
        calls: Mutex::new(Vec::new()),
        pages: Mutex::new(VecDeque::new()),
    }
}

impl FakeSource {
    pub(crate) fn serving(self, page: DailySpendPage) -> Self {
        self.pages.lock().unwrap().push_back(Ok(page));
        self
    }

    pub(crate) fn failing(self, error: UsageChatError) -> Self {
        self.pages.lock().unwrap().push_back(Err(error));
        self
    }

    pub(crate) fn calls(&self) -> Vec<SourceCall> {
        self.calls.lock().unwrap().clone()
    }

    fn take(
        &self,
        call: SourceCall,
    ) -> impl Future<Output = Result<DailySpendPage, UsageChatError>> + Send {
        async move {
            self.calls.lock().unwrap().push(call);
            self.pages
                .lock()
                .unwrap()
                .pop_front()
                .expect("scripted page")
        }
    }
}

impl UsageDataSource for FakeSource {
    fn global(
        &self,
        start_date: &str,
        end_date: &str,
        user_id: Option<&str>,
    ) -> impl Future<Output = Result<DailySpendPage, UsageChatError>> + Send {
        self.take(SourceCall::Global {
            start_date: start_date.to_owned(),
            end_date: end_date.to_owned(),
            user_id: user_id.map(str::to_owned),
        })
    }

    fn teams(
        &self,
        start_date: &str,
        end_date: &str,
        team_ids: Option<&[String]>,
    ) -> impl Future<Output = Result<DailySpendPage, UsageChatError>> + Send {
        self.take(SourceCall::Teams {
            start_date: start_date.to_owned(),
            end_date: end_date.to_owned(),
            team_ids: team_ids.map(<[String]>::to_vec),
        })
    }

    fn tags(
        &self,
        start_date: &str,
        end_date: &str,
        tags: Option<&[String]>,
    ) -> impl Future<Output = Result<DailySpendPage, UsageChatError>> + Send {
        self.take(SourceCall::Tags {
            start_date: start_date.to_owned(),
            end_date: end_date.to_owned(),
            tags: tags.map(<[String]>::to_vec),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AggregatedCall {
    pub(crate) table: DailyActivityTable,
    pub(crate) entity_id_field: String,
    pub(crate) entity_ids: Option<Vec<String>>,
    pub(crate) api_keys: Option<Vec<String>>,
    pub(crate) exclude_entity_ids: Vec<String>,
    pub(crate) start_date: String,
    pub(crate) end_date: String,
    pub(crate) model: Option<String>,
    pub(crate) timezone_offset_minutes: Option<i64>,
    pub(crate) include_current_utc_day: bool,
}

impl AggregatedCall {
    pub(crate) fn from_input(input: &DailyActivityScopeInput) -> Self {
        Self {
            table: input.table,
            entity_id_field: input.entity_id_field.clone(),
            entity_ids: input.entity_ids.clone(),
            api_keys: input.api_keys.clone(),
            exclude_entity_ids: input.exclude_entity_ids.clone(),
            start_date: input.start_date.clone(),
            end_date: input.end_date.clone(),
            model: input.model.clone(),
            timezone_offset_minutes: input.timezone_offset_minutes,
            include_current_utc_day: input.include_current_utc_day,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PagedCall {
    pub(crate) table: DailyActivityTable,
    pub(crate) entity_id_field: String,
    pub(crate) entity_id: Option<ScopeIds>,
    pub(crate) exclude_entity_ids: Vec<String>,
    pub(crate) start_date: Option<String>,
    pub(crate) end_date: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) api_key: Option<ScopeIds>,
    pub(crate) page: i64,
    pub(crate) page_size: i64,
    pub(crate) timezone_offset_minutes: Option<i64>,
    pub(crate) include_current_utc_day: bool,
    pub(crate) entity_metadata_absent: bool,
    pub(crate) callbacks_absent: bool,
}

pub(crate) struct RecordingRepository {
    pub(crate) aggregated: Mutex<Vec<AggregatedCall>>,
    pub(crate) paged: Mutex<Vec<PagedCall>>,
    pub(crate) pages: Mutex<VecDeque<Result<DailySpendPage, UsageChatError>>>,
}

pub(crate) fn recording_repository() -> RecordingRepository {
    RecordingRepository {
        aggregated: Mutex::new(Vec::new()),
        paged: Mutex::new(Vec::new()),
        pages: Mutex::new(VecDeque::new()),
    }
}

impl RecordingRepository {
    pub(crate) fn serving(self, page: DailySpendPage) -> Self {
        self.pages.lock().unwrap().push_back(Ok(page));
        self
    }

    pub(crate) fn aggregated_calls(&self) -> Vec<AggregatedCall> {
        self.aggregated.lock().unwrap().clone()
    }

    pub(crate) fn paged_calls(&self) -> Vec<PagedCall> {
        self.paged.lock().unwrap().clone()
    }
}

impl UsageActivityRepository for RecordingRepository {
    fn aggregated(
        &self,
        scope: &DailyActivityScope,
    ) -> impl Future<Output = Result<DailySpendPage, UsageChatError>> + Send {
        let call = AggregatedCall::from_input(scope.as_ref());
        async move {
            self.aggregated.lock().unwrap().push(call);
            self.pages
                .lock()
                .unwrap()
                .pop_front()
                .expect("scripted page")
        }
    }

    fn page(
        &self,
        request: &DailyActivityPageRequest<'_>,
    ) -> impl Future<Output = Result<DailySpendPage, UsageChatError>> + Send {
        let call = PagedCall {
            table: request.table,
            entity_id_field: request.entity_id_field.to_owned(),
            entity_id: request.entity_id.clone(),
            exclude_entity_ids: request.exclude_entity_ids.clone(),
            start_date: request.start_date.map(str::to_owned),
            end_date: request.end_date.map(str::to_owned),
            model: request.model.map(str::to_owned),
            api_key: request.api_key.clone(),
            page: request.page,
            page_size: request.page_size,
            timezone_offset_minutes: request.timezone_offset_minutes,
            include_current_utc_day: request.include_current_utc_day,
            entity_metadata_absent: request.entity_metadata.is_none(),
            callbacks_absent: request.callbacks.metadata_metrics.is_none()
                && request.callbacks.resolve_entity_metadata.is_none(),
        };
        async move {
            self.paged.lock().unwrap().push(call);
            self.pages
                .lock()
                .unwrap()
                .pop_front()
                .expect("scripted page")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FindCall {
    pub(crate) table: DailyActivityTable,
    pub(crate) skip: i64,
    pub(crate) take: i64,
}

pub(crate) struct RecordingDatabase {
    pub(crate) queries: Mutex<Vec<SqlQuery>>,
    pub(crate) finds: Mutex<Vec<FindCall>>,
    pub(crate) counts: Mutex<Vec<DailyActivityTable>>,
}

pub(crate) fn recording_database() -> RecordingDatabase {
    RecordingDatabase {
        queries: Mutex::new(Vec::new()),
        finds: Mutex::new(Vec::new()),
        counts: Mutex::new(Vec::new()),
    }
}

impl RecordingDatabase {
    pub(crate) fn queries(&self) -> Vec<SqlQuery> {
        self.queries.lock().unwrap().clone()
    }

    pub(crate) fn finds(&self) -> Vec<FindCall> {
        self.finds.lock().unwrap().clone()
    }

    pub(crate) fn counts(&self) -> Vec<DailyActivityTable> {
        self.counts.lock().unwrap().clone()
    }
}

impl DailyActivityDatabase for RecordingDatabase {
    fn query_raw<'database>(
        &'database self,
        query: &'database SqlQuery,
    ) -> DatabaseFuture<'database, Option<Vec<DatabaseRow>>> {
        self.queries.lock().unwrap().push(query.clone());
        Box::pin(ready(Ok(Some(Vec::new()))))
    }

    fn active_tokens<'database>(
        &'database self,
        _tokens: &'database [String],
    ) -> DatabaseFuture<'database, Vec<VerificationTokenRow>> {
        Box::pin(ready(Ok(Vec::new())))
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
        self.counts.lock().unwrap().push(table);
        Box::pin(ready(Ok(0)))
    }

    fn find_daily<'database>(
        &'database self,
        table: DailyActivityTable,
        _predicate: &'database SqlQuery,
        skip: i64,
        take: i64,
    ) -> DatabaseFuture<'database, Vec<DailyActivityRow>> {
        self.finds
            .lock()
            .unwrap()
            .push(FindCall { table, skip, take });
        Box::pin(ready(Ok(Vec::new())))
    }
}

pub(crate) struct NoReads;

impl DailyActivityProxyReads for NoReads {
    fn recover_key_metadata(
        &self,
        resolved: BTreeMap<String, KeyMetadataRow>,
        _api_keys: &BTreeSet<String>,
        _window: Option<SpendLogsWindow>,
    ) -> impl Future<Output = BTreeMap<String, KeyMetadataRow>> + Send {
        ready(resolved)
    }
}

pub(crate) fn concrete_repository(
    database: RecordingDatabase,
) -> DailyActivityRepository<RecordingDatabase, NoReads, fn() -> DateTime<Utc>> {
    DailyActivityRepository {
        database,
        proxy_reads: NoReads,
        clock: now,
    }
}

pub(crate) const ERROR_FRAME: &str = concat!(
    "data: {\"type\": \"error\", \"message\": ",
    "\"An internal error occurred. Please try again.\"}\n\n"
);

pub(crate) fn status_frame(message: &str) -> String {
    format!("data: {{\"type\": \"status\", \"message\": \"{message}\"}}\n\n")
}

pub(crate) fn chunk_frame(content: &str) -> String {
    format!("data: {{\"type\": \"chunk\", \"content\": \"{content}\"}}\n\n")
}

pub(crate) fn done_frame() -> String {
    "data: {\"type\": \"done\"}\n\n".to_owned()
}

pub(crate) fn frame_tool(
    name: &str,
    label: &str,
    arguments: &[(&str, &str)],
    status: &str,
) -> String {
    let rendered: Vec<String> = arguments
        .iter()
        .map(|(key, value)| format!("\"{key}\": \"{value}\""))
        .collect();
    format!(
        "data: {{\"type\": \"tool_call\", \"tool_name\": \"{name}\", \"tool_label\": \"{label}\", \"arguments\": {{{}}}, \"status\": \"{status}\"}}\n\n",
        rendered.join(", ")
    )
}

pub(crate) fn today() -> NaiveDate {
    on(2026, 11, 16)
}

pub(crate) fn chat_parts(
    model: FakeModel,
    data: FakeSource,
) -> UsageChatParts<FakeModel, FakeSource, fn() -> NaiveDate> {
    UsageChatParts {
        model,
        data,
        clock: today,
    }
}

pub(crate) fn chat_request<'text>(
    messages: &'text [ChatMessage],
    model: Option<&'text str>,
    user_id: Option<&'text str>,
    is_admin: bool,
) -> UsageChatRequest<'text> {
    UsageChatRequest {
        messages,
        model,
        user_id,
        is_admin,
    }
}

pub(crate) async fn drive<Model, Data, Clock>(
    parts: &UsageChatParts<Model, Data, Clock>,
    request: &UsageChatRequest<'_>,
) -> Vec<String>
where
    Model: UsageChatModel,
    Data: UsageDataSource,
    Clock: Fn() -> NaiveDate + Sync,
{
    let (sink, mut receiver) = mpsc::channel(128);
    stream_usage_ai_chat(parts, request, &sink).await;
    drop(sink);
    let mut frames = Vec::new();
    while let Some(frame) = receiver.recv().await {
        frames.push(frame);
    }
    frames
}

pub(crate) fn require_send<Value: Send>(value: Value) -> Value {
    value
}

pub(crate) fn spend_of(index: usize) -> String {
    format!("{}.0", 50_usize.saturating_sub(index))
}

pub(crate) fn args(entries: &[(&str, &str)]) -> IndexMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}
