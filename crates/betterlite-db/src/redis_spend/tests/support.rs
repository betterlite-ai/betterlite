use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Write};
use std::str::from_utf8;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use betterlite_redis::list_operations::error::RedisListOperationsError;
use rust_decimal::Decimal;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::daily_spend_sql::values::SpendColumnValue;
use crate::redis_spend::buffer::RedisUpdateBuffer;
use crate::redis_spend::codec::decode_log_record;
use crate::redis_spend::events::{RedisBufferEvent, RedisBufferEventSink};
use crate::redis_spend::lists::{
    BufferPop, BufferPush, BufferReply, SpendBufferListError, SpendBufferLists, SpendBufferPush,
    SpendListFuture,
};
use crate::redis_spend::log_row::SpendLogRow;
use crate::redis_spend::publication::{RedisSpendPublisher, SpendStaging};
use crate::spend_queues::daily::{DailySpendUpdate, DailySpendUpdateQueue};
use crate::spend_queues::entity::{
    EntityType, SpendAmount, SpendUpdateQueueItem, SpendUpdateTransactions,
};
use crate::spend_queues::events::NoSpendQueueEventSink;
use crate::window_queue::WindowSpendTransaction;
use crate::window_queue::queue::WindowSpendUpdateQueue;

#[derive(Debug, Clone, Default)]
pub(crate) struct Pushed {
    pub key: String,
    pub values: Vec<String>,
    pub max_len: i64,
}

// A producer that refills its bounded queue while the buffer is publishing, so the restore that follows meets a queue that is full again.
pub(crate) enum FillOnPush {
    Daily(Arc<DailySpendUpdateQueue>, DailySpendUpdate),
    Window(Arc<WindowSpendUpdateQueue>, WindowSpendTransaction),
}

impl FillOnPush {
    async fn apply(&self) {
        match self {
            Self::Daily(queue, update) => queue.add_update(update.clone()).await.unwrap(),
            Self::Window(queue, transaction) => {
                queue.add_update(transaction.clone()).await.unwrap()
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct FakeLists {
    state: Mutex<State>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Failure {
    #[default]
    None,
    Read,
    Trim,
    Push,
}

#[derive(Default)]
struct State {
    pops: VecDeque<BufferReply>,
    pipelines: VecDeque<Vec<BufferReply>>,
    failure: Failure,
    trim_size: i64,
    popped: Vec<(String, Option<i64>)>,
    piped: Vec<Vec<BufferPop>>,
    pushed: Vec<Pushed>,
    pushes: Vec<Vec<BufferPush>>,
    singles: Vec<(String, Vec<String>)>,
    push_lengths: VecDeque<Vec<i64>>,
    single_length: i64,
    fill_on_push: Option<FillOnPush>,
}

impl FakeLists {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn script_pop(&self, reply: BufferReply) {
        self.lock().pops.push_back(reply);
    }

    pub(crate) fn script_pipeline(&self, replies: Vec<BufferReply>) {
        self.lock().pipelines.push_back(replies);
    }

    pub(crate) fn fail_reads(&self) {
        self.lock().failure = Failure::Read;
    }

    pub(crate) fn fail_trim(&self) {
        self.lock().failure = Failure::Trim;
    }

    pub(crate) fn trim_size(&self, size: i64) {
        self.lock().trim_size = size;
    }

    pub(crate) fn script_push_lengths(&self, lengths: Vec<i64>) {
        self.lock().push_lengths.push_back(lengths);
    }

    pub(crate) fn single_length(&self, length: i64) {
        self.lock().single_length = length;
    }

    pub(crate) fn fail_push(&self) {
        self.lock().failure = Failure::Push;
    }

    pub(crate) fn allow_push(&self) {
        self.lock().failure = Failure::None;
    }

    pub(crate) fn fill_on_push(&self, fill: FillOnPush) {
        self.lock().fill_on_push = Some(fill);
    }

    pub(crate) fn popped(&self) -> Vec<(String, Option<i64>)> {
        self.lock().popped.clone()
    }

    pub(crate) fn piped(&self) -> Vec<Vec<BufferPop>> {
        self.lock().piped.clone()
    }

    pub(crate) fn pushed(&self) -> Vec<Pushed> {
        self.lock().pushed.clone()
    }

    pub(crate) fn pushed_pipelines(&self) -> Vec<Vec<BufferPush>> {
        self.lock().pushes.clone()
    }

    pub(crate) fn single_pushes(&self) -> Vec<(String, Vec<String>)> {
        self.lock().singles.clone()
    }

    // Records the pipeline, then lets a producer refill the bounded queue before the caller restores the drained cohort.
    async fn pipeline_push(
        &self,
        operations: &[BufferPush],
    ) -> Result<Vec<i64>, SpendBufferListError> {
        let pending_fill = {
            let mut state = self.lock();
            state.pushes.push(operations.to_vec());
            state.fill_on_push.take()
        };
        if let Some(fill) = pending_fill {
            fill.apply().await;
        }
        pipeline_result(&mut self.lock())
    }
}

impl SpendBufferLists for FakeLists {
    fn pop<'call>(
        &'call self,
        key: &'call str,
        count: Option<i64>,
    ) -> SpendListFuture<'call, BufferReply> {
        Box::pin(async move { pop_once(&mut self.lock(), key, count) })
    }

    fn pop_pipeline<'call>(
        &'call self,
        operations: &'call [BufferPop],
    ) -> SpendListFuture<'call, Vec<BufferReply>> {
        Box::pin(async move {
            let mut state = self.lock();
            state.piped.push(operations.to_vec());
            if state.failure == Failure::Read {
                return Err(list_failure("async_lpop_pipeline"));
            }
            Ok(state.pipelines.pop_front().unwrap_or_default())
        })
    }

    fn push_and_trim<'call>(
        &'call self,
        key: &'call str,
        values: &'call [String],
        max_len: i64,
    ) -> SpendListFuture<'call, i64> {
        Box::pin(async move {
            let mut state = self.lock();
            state.pushed.push(Pushed {
                key: key.to_owned(),
                values: values.to_vec(),
                max_len,
            });
            if state.failure == Failure::Trim {
                return Err(list_failure("async_rpush_and_trim"));
            }
            Ok(state.trim_size)
        })
    }
}

impl SpendBufferPush for FakeLists {
    fn rpush<'call>(
        &'call self,
        key: &'call str,
        values: &'call [String],
    ) -> SpendListFuture<'call, i64> {
        Box::pin(async move { single_push(&mut self.lock(), key, values) })
    }

    fn rpush_pipeline<'call>(
        &'call self,
        operations: &'call [BufferPush],
    ) -> SpendListFuture<'call, Vec<i64>> {
        Box::pin(async move { self.pipeline_push(operations).await })
    }
}

fn pop_once(
    state: &mut State,
    key: &str,
    count: Option<i64>,
) -> Result<BufferReply, SpendBufferListError> {
    let reply = state.pops.pop_front();
    state.popped.push((key.to_owned(), count));
    if state.failure == Failure::Read {
        return Err(list_failure("async_lpop"));
    }
    Ok(reply.unwrap_or(BufferReply::Missing))
}

// A rejected push records nothing: the source's append either lands or fails whole.
fn single_push(
    state: &mut State,
    key: &str,
    values: &[String],
) -> Result<i64, SpendBufferListError> {
    if state.failure == Failure::Push {
        return Err(list_failure("async_rpush"));
    }
    state.singles.push((key.to_owned(), values.to_vec()));
    Ok(state.single_length)
}

fn pipeline_result(state: &mut State) -> Result<Vec<i64>, SpendBufferListError> {
    if state.failure == Failure::Push {
        return Err(list_failure("async_rpush_pipeline"));
    }
    Ok(state.push_lengths.pop_front().unwrap_or_default())
}

fn list_failure(operation: &'static str) -> SpendBufferListError {
    SpendBufferListError::Redis {
        operation,
        source: RedisListOperationsError::UnexpectedReply { operation },
    }
}

#[derive(Default)]
pub(crate) struct RecordingEvents(Mutex<Vec<RedisBufferEvent>>);

impl RecordingEvents {
    pub(crate) fn events(&self) -> Vec<RedisBufferEvent> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RedisBufferEventSink for RecordingEvents {
    fn emit(&self, event: RedisBufferEvent) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs(Arc<Mutex<String>>);

impl CapturedLogs {
    pub(crate) fn text(&self) -> String {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text =
            from_utf8(buf).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_str(text);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn captured_logs() -> (CapturedLogs, DefaultGuard) {
    let logs = CapturedLogs::default();
    let guard = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .set_default();
    (logs, guard)
}

pub(crate) fn attached(lists: &Arc<FakeLists>) -> RedisUpdateBuffer {
    let owned: Arc<FakeLists> = Arc::clone(lists);
    let port: Arc<dyn SpendBufferLists> = owned;
    RedisUpdateBuffer {
        lists: Some(port),
        enabled: true,
        dequeue_count: 5,
        events: Arc::new(RecordingEvents::default()),
    }
}

pub(crate) fn detached() -> RedisUpdateBuffer {
    RedisUpdateBuffer {
        lists: None,
        enabled: true,
        dequeue_count: 5,
        events: Arc::new(RecordingEvents::default()),
    }
}

pub(crate) fn buffer_with_sink(events: Arc<dyn RedisBufferEventSink>) -> RedisUpdateBuffer {
    RedisUpdateBuffer {
        lists: None,
        enabled: false,
        dequeue_count: 5,
        events,
    }
}

pub(crate) fn publisher(lists: &Arc<FakeLists>) -> RedisSpendPublisher {
    let (publisher, _events) = publisher_recording(lists);
    publisher
}

pub(crate) fn publisher_recording(
    lists: &Arc<FakeLists>,
) -> (RedisSpendPublisher, Arc<RecordingEvents>) {
    let events = Arc::new(RecordingEvents::default());
    let owned: Arc<FakeLists> = Arc::clone(lists);
    let port: Arc<dyn SpendBufferPush> = owned;
    let sink: Arc<dyn RedisBufferEventSink> = Arc::<RecordingEvents>::clone(&events);
    let publisher = RedisSpendPublisher {
        lists: Some(port),
        events: sink,
        staging: SpendStaging::default(),
    };
    (publisher, events)
}

pub(crate) fn detached_publisher() -> RedisSpendPublisher {
    RedisSpendPublisher {
        lists: None,
        events: Arc::new(RecordingEvents::default()),
        staging: SpendStaging::default(),
    }
}

// The queues a scheduler tick publishes from, each unbounded enough that only a test asking for a full one gets one.
pub(crate) fn daily_queue(
    capacity: i64,
    threshold: i64,
    flush_count: i64,
) -> DailySpendUpdateQueue {
    DailySpendUpdateQueue::new(
        capacity,
        threshold,
        flush_count,
        Arc::new(NoSpendQueueEventSink),
    )
    .unwrap()
}

pub(crate) fn window_queue(
    capacity: i64,
    threshold: i64,
    flush_count: i64,
) -> WindowSpendUpdateQueue {
    WindowSpendUpdateQueue::new(capacity, threshold, flush_count).unwrap()
}

pub(crate) fn amount(value: &str) -> SpendAmount {
    SpendAmount(value.parse::<Decimal>().unwrap())
}

pub(crate) fn queue_item(kind: EntityType, id: &str, value: &str) -> SpendUpdateQueueItem {
    SpendUpdateQueueItem {
        entity_type: Some(kind),
        entity_id: Some(id.to_owned()),
        response_cost: Some(amount(value)),
    }
}

pub(crate) fn transactions(user: &[(&str, &str)]) -> SpendUpdateTransactions {
    SpendUpdateTransactions {
        user_list_transactions: user
            .iter()
            .map(|(id, value)| ((*id).to_owned(), amount(value)))
            .collect(),
        end_user_list_transactions: BTreeMap::new(),
        key_list_transactions: BTreeMap::new(),
        team_list_transactions: BTreeMap::new(),
        team_member_list_transactions: BTreeMap::new(),
        org_list_transactions: BTreeMap::new(),
        org_member_list_transactions: BTreeMap::new(),
        project_list_transactions: BTreeMap::new(),
        tag_list_transactions: BTreeMap::new(),
        agent_list_transactions: BTreeMap::new(),
        model_access_group_spend: BTreeMap::new(),
    }
}

// One daily aggregation key with the six required metrics the merge insists on; the request counters stay zero so a fixture can be summed with itself without the test restating the totals.
pub(crate) fn daily_row(day: &str, spend: &str, prompt_tokens: i64) -> DailySpendUpdate {
    let mut row = BTreeMap::new();
    row.insert(
        "spend".to_owned(),
        SpendColumnValue::Decimal(spend.parse::<Decimal>().unwrap()),
    );
    row.insert(
        "prompt_tokens".to_owned(),
        SpendColumnValue::Int(prompt_tokens),
    );
    row.insert("completion_tokens".to_owned(), SpendColumnValue::Int(0));
    row.insert("api_requests".to_owned(), SpendColumnValue::Int(0));
    row.insert("successful_requests".to_owned(), SpendColumnValue::Int(0));
    row.insert("failed_requests".to_owned(), SpendColumnValue::Int(0));
    DailySpendUpdate::from([(day.to_owned(), row)])
}

// What aggregate_daily_rows makes of two daily_row payloads for the same day: the six required metrics summed and the nine optional ones the merge writes as zero.
pub(crate) fn merged_daily_row(day: &str, spend: &str, prompt_tokens: i64) -> DailySpendUpdate {
    let mut merged = daily_row(day, spend, prompt_tokens);
    if let Some(row) = merged.get_mut(day) {
        for metric in [
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
            "compression_saved_tokens",
            "compression_savings_spend",
            "prompt_caching_savings_spend",
            "gateway_injected_caching_savings_spend",
            "autorouter_savings_spend",
            "total_response_time_ms",
            "timed_requests",
        ] {
            row.insert(metric.to_owned(), SpendColumnValue::Int(0));
        }
    }
    merged
}

pub(crate) fn window_transaction(kind: &str, id: &str, spend: &str) -> WindowSpendTransaction {
    WindowSpendTransaction {
        entity_type: kind.to_owned(),
        entity_id: id.to_owned(),
        window_duration: "1h".to_owned(),
        window_start: "2024-01-01T00:00:00+00:00".to_owned(),
        spend: amount(spend),
        started_at: None,
    }
}

pub(crate) fn decoded_row(json: &str) -> SpendLogRow {
    decode_log_record(json.as_bytes()).unwrap().unwrap()
}

pub(crate) fn bytes(value: &str) -> Vec<u8> {
    value.as_bytes().to_vec()
}
