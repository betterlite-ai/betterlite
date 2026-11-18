#![cfg(test)]

use std::collections::VecDeque;
use std::io;
use std::str;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;

use betterlite_redis::list_operations::error::RedisListOperationsError;
use tokio_postgres::types::ToSql;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::gateway_request_tracking::commit_gateway_requests::CommitFuture;
use crate::gateway_request_tracking::commit_gateway_requests::GatewayRequestsDatabase;
use crate::gateway_request_tracking::gateway_request_redis::GatewayRequestRedisBuffer;
use crate::gateway_request_tracking::gateway_request_redis::GatewayRequestRedisError;
use crate::gateway_request_tracking::gateway_request_redis::GatewayRequestsLease;
use crate::gateway_request_tracking::gateway_request_redis::GatewayRequestsList;
use crate::gateway_request_tracking::gateway_request_redis::LeaseFuture;
use crate::gateway_request_tracking::gateway_request_redis::ListFuture;
use crate::gateway_requests::GatewayRequestKey;
use crate::gateway_requests::GatewayRequestSnapshot;
use crate::gateway_requests::gateway_request_counts::GatewayRequestCounts;

pub(crate) fn key(date: &str, category: &str, route: &str) -> GatewayRequestKey {
    GatewayRequestKey {
        date: date.to_owned(),
        category: category.to_owned(),
        route: route.to_owned(),
    }
}

pub(crate) fn counts(successful_requests: i64, failed_requests: i64) -> GatewayRequestCounts {
    GatewayRequestCounts {
        successful_requests,
        failed_requests,
    }
}

pub(crate) fn snapshot(rows: &[(&str, &str, &str, i64, i64)]) -> GatewayRequestSnapshot {
    rows.iter()
        .map(
            |(date, category, route, successful_requests, failed_requests)| {
                (
                    key(date, category, route),
                    counts(*successful_requests, *failed_requests),
                )
            },
        )
        .collect()
}

pub(crate) fn buffer_with(
    list: Arc<FakeList>,
    lease: Arc<FakeLease>,
    dequeue_count: usize,
) -> GatewayRequestRedisBuffer {
    GatewayRequestRedisBuffer {
        list,
        lease,
        dequeue_count,
    }
}

#[derive(Default)]
struct ListState {
    entries: VecDeque<String>,
    pushed: Vec<String>,
    take_calls: Vec<usize>,
    fail_take: bool,
    push_budget: Option<usize>,
}

#[derive(Default)]
pub(crate) struct FakeList {
    state: Mutex<ListState>,
}

impl FakeList {
    fn lock(&self) -> MutexGuard<'_, ListState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn with_entries(entries: &[&str]) -> Self {
        let mut state = ListState::default();
        state.entries = entries.iter().map(|entry| (*entry).to_owned()).collect();
        Self {
            state: Mutex::new(state),
        }
    }

    pub(crate) fn fail_push(&self) {
        self.fail_push_after(0);
    }

    pub(crate) fn fail_push_after(&self, successes: usize) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_budget = Some(successes);
    }

    pub(crate) fn fail_take(&self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .fail_take = true;
    }

    pub(crate) fn pushed(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pushed
            .clone()
    }

    pub(crate) fn take_calls(&self) -> Vec<usize> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take_calls
            .clone()
    }

    pub(crate) fn queued(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entries
            .iter()
            .cloned()
            .collect()
    }

    fn append_blocking(&self, entry: &str) -> Result<(), GatewayRequestRedisError> {
        let mut state = self.lock();
        if let Some(remaining) = state.push_budget {
            if remaining == 0 {
                return Err(list_failure("async_rpush"));
            }
            state.push_budget = Some(remaining.saturating_sub(1));
        }
        state.pushed.push(entry.to_owned());
        state.entries.push_back(entry.to_owned());
        Ok(())
    }

    fn take_blocking(&self, count: usize) -> Result<Vec<String>, GatewayRequestRedisError> {
        let mut state = self.lock();
        state.take_calls.push(count);
        if state.fail_take {
            return Err(list_failure("async_lpop"));
        }
        let available = count.min(state.entries.len());
        Ok(state.entries.drain(..available).collect())
    }
}

fn list_failure(operation: &'static str) -> GatewayRequestRedisError {
    GatewayRequestRedisError::List {
        operation,
        source: RedisListOperationsError::UnexpectedReply { operation },
    }
}

impl GatewayRequestsList for FakeList {
    fn append<'call>(&'call self, entry: &'call str) -> ListFuture<'call, ()> {
        Box::pin(async move { self.append_blocking(entry) })
    }

    fn take<'call>(&'call self, count: usize) -> ListFuture<'call, Vec<String>> {
        Box::pin(async move { self.take_blocking(count) })
    }
}

pub(crate) struct FakeLease {
    reply: Option<bool>,
    calls: Mutex<Vec<String>>,
}

impl FakeLease {
    pub(crate) fn granting() -> Self {
        Self {
            reply: Some(true),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn denying() -> Self {
        Self {
            reply: Some(false),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn unavailable() -> Self {
        Self {
            reply: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl GatewayRequestsLease for FakeLease {
    fn acquire<'call>(&'call self, job: &'call str) -> LeaseFuture<'call> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(job.to_owned());
            self.reply
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake database is unavailable")]
pub(crate) struct FakeDatabaseError;

#[derive(Default)]
struct DatabaseState {
    calls: usize,
    fail: bool,
    last_sql: Option<String>,
    last_parameters: usize,
}

#[derive(Default)]
pub(crate) struct FakeDatabase {
    state: Mutex<DatabaseState>,
}

impl FakeDatabase {
    pub(crate) fn failing() -> Self {
        let database = Self::default();
        database
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .fail = true;
        database
    }

    pub(crate) fn fail(&self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .fail = true;
    }

    pub(crate) fn calls(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .calls
    }

    pub(crate) fn last_sql(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last_sql
            .clone()
    }

    pub(crate) fn last_parameters(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last_parameters
    }
}

impl GatewayRequestsDatabase for FakeDatabase {
    type Error = FakeDatabaseError;

    fn execute_upsert<'call>(
        &'call self,
        sql: &'call str,
        parameters: Vec<&'call (dyn ToSql + Sync)>,
    ) -> CommitFuture<'call, u64, Self::Error> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.calls = state.calls.saturating_add(1);
            if state.fail {
                return Err(FakeDatabaseError);
            }
            state.last_sql = Some(sql.to_owned());
            state.last_parameters = parameters.len();
            Ok(u64::try_from(parameters.len()).unwrap_or_default())
        })
    }
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(crate) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        str::from_utf8(&bytes)
            .expect("the captured logs are UTF-8")
            .to_owned()
    }
}

impl io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
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
