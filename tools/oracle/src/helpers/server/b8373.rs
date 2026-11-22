use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite::startup_checks::presence::{DatabasePresence, RedisPresence};
use betterlite::startup_checks::transaction_buffer::check_transaction_buffer;
use betterlite::startup_checks::tuning_baseline::decode_tuning_baselines;
use betterlite::startup_checks::warnings::{
    warn_budget_without_db, warn_fail_closed_without_redis,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::coord_redis_boot::settings::CoordinationGeneralSettings;
use betterlite_server::proxy_request_contracts::coordination_redis::BoolOrText;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde::de::Error as _;
use serde_json::{Error as JsonError, Value, json};
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;

// the four pure startup-check subjects this operation exposes to LiteLLM
#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    TransactionBuffer {
        use_redis_transaction_buffer: BoolOrText,
        redis_configured: bool,
    },
    BudgetWarning {
        #[serde(with = "rust_decimal::serde::arbitrary_precision")]
        max_budget: Decimal,
        database_configured: bool,
    },
    FailClosedWarning {
        enabled: bool,
        redis_configured: bool,
    },
    BaselineRow {
        row: Value,
    },
}

pub(crate) struct Op;

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn first_line(&self) -> Option<String> {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let text = String::from_utf8(bytes).ok()?;
        text.lines()
            .find(|line| !line.is_empty())
            .map(str::to_owned)
    }
}

impl io::Write for LogBuffer {
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

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

// the warning functions only log, so the answer is the first line they wrote
fn warned_message<F>(emit: F) -> Value
where
    F: FnOnce(),
{
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_level(false)
        .with_writer(buffer.clone())
        .finish();
    let guard = set_default(subscriber);
    emit();
    drop(guard);
    buffer.first_line().map_or_else(
        || json!({"warned": false, "message": Value::Null}),
        |message| json!({"warned": true, "message": message}),
    )
}

const fn presence(configured: bool) -> RedisPresence {
    if configured {
        RedisPresence::Present
    } else {
        RedisPresence::Absent
    }
}

fn transaction_buffer(flag: BoolOrText, redis_configured: bool) -> Value {
    let settings = CoordinationGeneralSettings {
        coordination_redis: None,
        use_redis_transaction_buffer: Some(flag),
    };
    match check_transaction_buffer(&settings, presence(redis_configured)) {
        Ok(()) => json!({"raised": false, "message": Value::Null}),
        Err(error) => json!({"raised": true, "message": error.to_string()}),
    }
}

fn baseline_row(row: &Value) -> HelperResult {
    let stored: Json = serde_json::from_value(row.clone()).or_value_error()?;
    let decoded = decode_tuning_baselines(&stored).or_value_error()?;
    serde_json::to_value(&decoded).or_value_error()
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::TransactionBuffer {
                use_redis_transaction_buffer,
                redis_configured,
            } => Ok(transaction_buffer(
                use_redis_transaction_buffer.clone(),
                *redis_configured,
            )),
            Input::BudgetWarning {
                max_budget,
                database_configured,
            } => {
                let budget = max_budget.to_f64().ok_or_else(|| {
                    JsonError::custom(format!("max_budget {max_budget} has no f64 representation"))
                })?;
                let database = if *database_configured {
                    DatabasePresence::Present
                } else {
                    DatabasePresence::Absent
                };
                Ok(warned_message(|| {
                    warn_budget_without_db(Some(budget), database);
                }))
            }
            Input::FailClosedWarning {
                enabled,
                redis_configured,
            } => Ok(warned_message(|| {
                warn_fail_closed_without_redis(*enabled, presence(*redis_configured));
            })),
            Input::BaselineRow { row } => baseline_row(row),
        }
    }
}
