use std::collections::VecDeque;
use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use betterlite_db::daily_spend_sql::entities::{DailySpendEntity, SpendRow};
use betterlite_db::daily_spend_sql::values::SpendColumnValue;
use betterlite_db::key_user_writes::retry::WriteClock;
use betterlite_db::migration_ledger::lock::JitterSource;
use betterlite_db::spend_commit::daily::{
    ChunkFuture, DailyChunkCommit, DailyChunkFailure, DailyChunkWriter,
};
use betterlite_db::spend_commit::phase::CommitPhase;
use betterlite_db::spend_commit::policy::{SpendFailure, SpendRetryPolicy, SpendUpdateFailure};
use betterlite_db::spend_log_storage::rows::writer::{SpendLogFailureEvent, SpendLogFailureSink};
use betterlite_db::spend_queues::daily::DailySpendUpdate;
use betterlite_db::tool_spend::RetryPause;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    retries: u32,
    rows: usize,
    #[serde(default)]
    script: Vec<ScriptToken>,
}

// One scripted outcome of the next daily statement, named after the LiteLLM exception class it stands for.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScriptToken {
    Ok,
    ConnectError,
    ReadTimeout,
    ReadError,
    DataRejected,
    Deadlock,
    UnknownSqlstate,
    OtherFailure,
}

impl ScriptToken {
    const fn name(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::ConnectError => "connect_error",
            Self::ReadTimeout => "read_timeout",
            Self::ReadError => "read_error",
            Self::DataRejected => "data_rejected",
            Self::Deadlock => "deadlock",
            Self::UnknownSqlstate => "unknown_sqlstate",
            Self::OtherFailure => "other_failure",
        }
    }

    // What each LiteLLM exception class means to the two replay predicates.
    const fn failure(self) -> SpendFailure {
        match self {
            Self::Ok => SpendFailure::Other,
            Self::ConnectError => SpendFailure::NotSent,
            Self::ReadTimeout | Self::ReadError => SpendFailure::Ambiguous,
            Self::DataRejected => SpendFailure::Rejected,
            Self::Deadlock => SpendFailure::Deadlock,
            Self::UnknownSqlstate | Self::OtherFailure => SpendFailure::Other,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{name}")]
struct ScriptedFailure {
    name: &'static str,
    failure: SpendFailure,
}

impl SpendUpdateFailure for ScriptedFailure {
    fn spend_failure(&self) -> SpendFailure {
        self.failure
    }
}

impl DailyChunkFailure for ScriptedFailure {}

struct ScriptedWriter {
    script: Mutex<VecDeque<ScriptToken>>,
    attempts: AtomicUsize,
}

impl ScriptedWriter {
    fn new(script: &[ScriptToken]) -> Self {
        Self {
            script: Mutex::new(script.iter().copied().collect()),
            attempts: AtomicUsize::new(0),
        }
    }
}

impl DailyChunkWriter for ScriptedWriter {
    type Error = ScriptedFailure;

    fn write_chunk<'writer>(
        &'writer self,
        _entity: DailySpendEntity,
        _rows: &'writer [SpendRow],
        _phase: &'writer CommitPhase,
    ) -> ChunkFuture<'writer, ScriptedFailure> {
        Box::pin(async move {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            let mut script = self.script.lock().unwrap_or_else(PoisonError::into_inner);
            let token = script.pop_front().unwrap_or(ScriptToken::Ok);
            match token {
                ScriptToken::Ok => Ok(()),
                failed => Err(ScriptedFailure {
                    name: failed.name(),
                    failure: failed.failure(),
                }),
            }
        })
    }
}

struct ScriptedClock;

impl WriteClock for ScriptedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::<Utc>::UNIX_EPOCH
    }

    fn instant(&self) -> Instant {
        Instant::now()
    }
}

struct NoJitter;

impl JitterSource for NoJitter {
    fn uniform_nanos(&self, _upper_exclusive_nanos: u64) -> u64 {
        0
    }
}

struct NoPause;

impl RetryPause for NoPause {
    fn pause(&self, _delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(std::future::ready(()))
    }
}

#[derive(Default)]
struct CountingSink {
    reported: AtomicUsize,
}

impl SpendLogFailureSink for CountingSink {
    fn emit(&self, _event: SpendLogFailureEvent) {
        self.reported.fetch_add(1, Ordering::SeqCst);
    }
}

// The same rows both sides build: a fixed date, the row's entity id and one dollar of spend.
fn pending_rows(count: usize) -> DailySpendUpdate {
    (0_usize..count)
        .map(|index| {
            let values: SpendRow = [
                ("date", SpendColumnValue::Text("2026-01-02".to_owned())),
                (
                    "user_id",
                    SpendColumnValue::Text(format!("user-{index:03}")),
                ),
                ("api_key", SpendColumnValue::Text("key-1".to_owned())),
                ("model", SpendColumnValue::Text("model-1".to_owned())),
                (
                    "custom_llm_provider",
                    SpendColumnValue::Text("openai".to_owned()),
                ),
                ("spend", SpendColumnValue::Decimal(Decimal::ONE)),
            ]
            .into_iter()
            .map(|(field, value)| (field.to_owned(), value))
            .collect();
            (format!("row-{index:03}"), values)
        })
        .collect()
}

async fn run_call(call: &Call) -> Value {
    let writer = ScriptedWriter::new(&call.script);
    let clock = ScriptedClock;
    let jitter = NoJitter;
    let pause = NoPause;
    let sink = CountingSink::default();
    let policy = SpendRetryPolicy {
        retries: call.retries,
        clock: &clock,
        jitter: &jitter,
        pause: &pause,
        failures: &sink,
    };
    let commit = DailyChunkCommit {
        writer: &writer,
        policy,
        phase: Arc::new(CommitPhase::new()),
    };
    let mut pending = pending_rows(call.rows);
    let error = match commit
        .update_daily_spend(DailySpendEntity::User, &mut pending)
        .await
    {
        Ok(()) => String::new(),
        Err(failure) => failure.to_string(),
    };
    json!({
        "attempts": writer.attempts.load(Ordering::SeqCst),
        "remaining": pending.keys().cloned().collect::<Vec<_>>(),
        "reported": sink.reported.load(Ordering::SeqCst),
        "error": error,
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(failure)?;
        runtime.block_on(async {
            let mut calls = Vec::with_capacity(input.calls.len());
            for call in &input.calls {
                calls.push(run_call(call).await);
            }
            Ok(json!({"calls": calls}))
        })
    }
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "CommitRemainder".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
