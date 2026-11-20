use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Write};
use std::str::from_utf8;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use betterlite_redis::list_operations::error::RedisListOperationsError;
use rust_decimal::Decimal;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::redis_spend::buffer::RedisUpdateBuffer;
use crate::redis_spend::codec::decode_log_record;
use crate::redis_spend::events::{RedisBufferEvent, RedisBufferEventSink};
use crate::redis_spend::lists::{
    BufferPop, BufferReply, SpendBufferListError, SpendBufferLists, SpendListFuture,
};
use crate::redis_spend::log_row::SpendLogRow;
use crate::spend_queues::entity::{SpendAmount, SpendUpdateTransactions};

#[derive(Debug, Clone, Default)]
pub(super) struct Pushed {
    pub key: String,
    pub values: Vec<String>,
    pub max_len: i64,
}

#[derive(Default)]
pub(super) struct FakeLists {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    pops: VecDeque<BufferReply>,
    pipelines: VecDeque<Vec<BufferReply>>,
    fail_reads: bool,
    fail_trim: bool,
    trim_size: i64,
    popped: Vec<(String, Option<i64>)>,
    piped: Vec<Vec<BufferPop>>,
    pushed: Vec<Pushed>,
}

impl FakeLists {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn script_pop(&self, reply: BufferReply) {
        self.lock().pops.push_back(reply);
    }

    pub(super) fn script_pipeline(&self, replies: Vec<BufferReply>) {
        self.lock().pipelines.push_back(replies);
    }

    pub(super) fn fail_reads(&self) {
        self.lock().fail_reads = true;
    }

    pub(super) fn fail_trim(&self) {
        self.lock().fail_trim = true;
    }

    pub(super) fn trim_size(&self, size: i64) {
        self.lock().trim_size = size;
    }

    pub(super) fn popped(&self) -> Vec<(String, Option<i64>)> {
        self.lock().popped.clone()
    }

    pub(super) fn piped(&self) -> Vec<Vec<BufferPop>> {
        self.lock().piped.clone()
    }

    pub(super) fn pushed(&self) -> Vec<Pushed> {
        self.lock().pushed.clone()
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
            if state.fail_reads {
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
            if state.fail_trim {
                return Err(list_failure("async_rpush_and_trim"));
            }
            Ok(state.trim_size)
        })
    }
}

fn pop_once(
    state: &mut State,
    key: &str,
    count: Option<i64>,
) -> Result<BufferReply, SpendBufferListError> {
    let reply = state.pops.pop_front();
    state.popped.push((key.to_owned(), count));
    if state.fail_reads {
        return Err(list_failure("async_lpop"));
    }
    Ok(reply.unwrap_or(BufferReply::Missing))
}

fn list_failure(operation: &'static str) -> SpendBufferListError {
    SpendBufferListError::Redis {
        operation,
        source: RedisListOperationsError::UnexpectedReply { operation },
    }
}

#[derive(Default)]
pub(super) struct RecordingEvents(Mutex<Vec<RedisBufferEvent>>);

impl RecordingEvents {
    pub(super) fn events(&self) -> Vec<RedisBufferEvent> {
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
pub(super) struct CapturedLogs(Arc<Mutex<String>>);

impl CapturedLogs {
    pub(super) fn text(&self) -> String {
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

pub(super) fn captured_logs() -> (CapturedLogs, DefaultGuard) {
    let logs = CapturedLogs::default();
    let guard = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .set_default();
    (logs, guard)
}

pub(super) fn attached(lists: &Arc<FakeLists>) -> RedisUpdateBuffer {
    let owned: Arc<FakeLists> = Arc::clone(lists);
    let port: Arc<dyn SpendBufferLists> = owned;
    RedisUpdateBuffer {
        lists: Some(port),
        enabled: true,
        dequeue_count: 5,
        events: Arc::new(RecordingEvents::default()),
    }
}

pub(super) fn detached() -> RedisUpdateBuffer {
    RedisUpdateBuffer {
        lists: None,
        enabled: true,
        dequeue_count: 5,
        events: Arc::new(RecordingEvents::default()),
    }
}

pub(super) fn buffer_with_sink(events: Arc<dyn RedisBufferEventSink>) -> RedisUpdateBuffer {
    RedisUpdateBuffer {
        lists: None,
        enabled: false,
        dequeue_count: 5,
        events,
    }
}

pub(super) fn amount(value: &str) -> SpendAmount {
    SpendAmount(value.parse::<Decimal>().unwrap())
}

pub(super) fn transactions(user: &[(&str, &str)]) -> SpendUpdateTransactions {
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

pub(super) fn decoded_row(json: &str) -> SpendLogRow {
    decode_log_record(json.as_bytes()).unwrap().unwrap()
}

pub(super) fn bytes(value: &str) -> Vec<u8> {
    value.as_bytes().to_vec()
}
