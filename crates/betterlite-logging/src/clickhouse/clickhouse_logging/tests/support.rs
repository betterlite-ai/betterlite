use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::iter::repeat_n;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use secrecy::SecretString;
use tokio::sync::Notify;
use tokio::task::yield_now;

use super::super::payload::CallbackKwargs;
use super::super::settings::ClickHouseSettings;
use super::super::storage::ClickHouseStorage;
use crate::trace_types::SpendLogRecord;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InsertOutcome {
    Succeed,
    Fail,
}

#[derive(Debug, thiserror::Error)]
#[error("the fake ClickHouse insert failed")]
pub(crate) struct FakeError;

#[derive(Clone, Debug)]
pub(crate) struct InsertCall {
    pub table: String,
    pub rows: Vec<SpendLogRecord>,
}

struct FakeState {
    calls: Mutex<Vec<InsertCall>>,
    outcomes: Mutex<VecDeque<InsertOutcome>>,
    gate: Mutex<Option<Gate>>,
    schema_calls: Mutex<Vec<(u32, u32)>>,
    schema_fails: AtomicBool,
}

#[derive(Clone)]
pub(crate) struct Gate {
    entered: Arc<AtomicBool>,
    permit: Arc<Notify>,
}

impl Gate {
    pub(crate) async fn entered(&self) {
        while !self.entered.load(Ordering::SeqCst) {
            yield_now().await;
        }
    }

    pub(crate) fn release(&self) {
        self.permit.notify_one();
    }
}

#[derive(Clone, Default)]
pub(crate) struct FakeStorage {
    state: Arc<FakeState>,
}

impl FakeStorage {
    pub(crate) fn fail_next(&self, count: usize) {
        let mut outcomes = self
            .state
            .outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        outcomes.extend(repeat_n(InsertOutcome::Fail, count));
    }

    pub(crate) fn fail_schema(&self) {
        self.state.schema_fails.store(true, Ordering::SeqCst);
    }

    pub(crate) fn hold(&self) -> Gate {
        let gate = Gate {
            entered: Arc::new(AtomicBool::new(false)),
            permit: Arc::new(Notify::new()),
        };
        *self
            .state
            .gate
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(gate.clone());
        gate
    }

    pub(crate) fn calls(&self) -> Vec<InsertCall> {
        self.state
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn insert_count(&self) -> usize {
        self.state
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(crate) fn sent_request_ids(&self) -> Vec<Vec<String>> {
        self.calls()
            .iter()
            .map(|call| call.rows.iter().map(|row| row.request_id.clone()).collect())
            .collect()
    }

    pub(crate) fn schema_calls(&self) -> Vec<(u32, u32)> {
        self.state
            .schema_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Default for FakeState {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            outcomes: Mutex::new(VecDeque::new()),
            gate: Mutex::new(None),
            schema_calls: Mutex::new(Vec::new()),
            schema_fails: AtomicBool::new(false),
        }
    }
}

impl ClickHouseStorage for FakeStorage {
    type Error = FakeError;

    fn insert_rows(
        &self,
        table: &str,
        rows: &[SpendLogRecord],
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.state
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(InsertCall {
                table: table.to_owned(),
                rows: rows.to_vec(),
            });
        let outcome = self
            .state
            .outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(InsertOutcome::Succeed);
        let hold = self
            .state
            .gate
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        async move {
            if let Some(gate) = hold {
                gate.entered.store(true, Ordering::SeqCst);
                gate.permit.notified().await;
            }
            match outcome {
                InsertOutcome::Succeed => Ok(()),
                InsertOutcome::Fail => Err(FakeError),
            }
        }
    }

    fn ensure_schema(
        &self,
        trace_retention_days: u32,
        spend_log_retention_days: u32,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.state
            .schema_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((trace_retention_days, spend_log_retention_days));
        let fails = self.state.schema_fails.load(Ordering::SeqCst);
        async move {
            if fails {
                return Err(FakeError);
            }
            Ok(())
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Captured(pub(crate) Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn settings(
    batch_size: usize,
    flush_interval: Duration,
    max_retries: usize,
) -> ClickHouseSettings {
    ClickHouseSettings {
        database: "litellm".to_owned(),
        url: SecretString::from(String::new()),
        batch_size,
        flush_interval,
        max_buffered_rows: 200_000,
        max_retries,
    }
}

pub(crate) fn row(request_id: &str) -> SpendLogRecord {
    SpendLogRecord {
        request_id: request_id.to_owned(),
        response_id: request_id.to_owned(),
        call_type: "acompletion".to_owned(),
        api_key: String::new(),
        key_alias: String::new(),
        team_id: String::new(),
        team_alias: String::new(),
        organization_id: String::new(),
        user: String::new(),
        end_user: String::new(),
        model: String::new(),
        model_group: String::new(),
        model_id: String::new(),
        custom_llm_provider: String::new(),
        api_base: String::new(),
        spend: rust_decimal::Decimal::ZERO,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        start_time: 0,
        end_time: 0,
        completion_start_time: None,
        status: String::new(),
        error_str: String::new(),
        cache_hit: false,
        session_id: String::new(),
        trace_id: String::new(),
        span_id: String::new(),
        request_tags: Vec::new(),
        metadata: String::new(),
        messages: String::new(),
        response: String::new(),
    }
}

pub(crate) fn kwargs(json: &str) -> CallbackKwargs {
    serde_json::from_str(json).expect("the fixture kwargs decode")
}

pub(crate) async fn wait_for_inserts(storage: &FakeStorage, expected: usize) {
    for _ in 0_usize..1_000 {
        if storage.insert_count() >= expected {
            return;
        }
        yield_now().await;
    }
    panic!("the expected ClickHouse insert never happened");
}
