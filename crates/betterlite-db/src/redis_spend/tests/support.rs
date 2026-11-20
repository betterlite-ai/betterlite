use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use betterlite_redis::list_operations::error::RedisListOperationsError;

use crate::redis_spend::buffer::RedisUpdateBuffer;
use crate::redis_spend::codec::decode_log_record;
use crate::redis_spend::events::{RedisBufferEvent, RedisBufferEventSink};
use crate::redis_spend::lists::{
    BufferPop, BufferReply, SpendBufferListError, SpendBufferLists, SpendListFuture,
};
use crate::redis_spend::log_row::SpendLogRow;

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
        Box::pin(async move {
            let mut state = self.lock();
            state.popped.push((key.to_owned(), count));
            if state.fail_reads {
                return Err(list_failure("async_lpop"));
            }
            Ok(state.pops.pop_front().unwrap_or(BufferReply::Missing))
        })
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

pub(super) fn attached(lists: &Arc<FakeLists>) -> RedisUpdateBuffer {
    let port: Arc<dyn SpendBufferLists> = Arc::clone(lists);
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

pub(super) fn decoded_row(json: &str) -> SpendLogRow {
    decode_log_record(json.as_bytes()).unwrap().unwrap()
}

pub(super) fn bytes(value: &str) -> Vec<u8> {
    value.as_bytes().to_vec()
}
