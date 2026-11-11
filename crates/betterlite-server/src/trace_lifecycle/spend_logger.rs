use std::convert::Infallible;
use std::error::Error;
use std::future::Future;
use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::clickhouse::clickhouse_logging::batch_logger::ClickHouseBatchLogger;
use betterlite_logging::clickhouse::clickhouse_logging::context::LensAnalysis;
use betterlite_logging::clickhouse::clickhouse_logging::payload::CallbackKwargs;
use betterlite_logging::clickhouse::clickhouse_logging::schema::SPEND_LOGS_TABLE;
use betterlite_logging::clickhouse::clickhouse_logging::settings::ClickHouseSettings;
use betterlite_logging::clickhouse::clickhouse_logging::spend_logger::{
    CallbackEvent, ClickHouseSpendLogger,
};
use betterlite_logging::clickhouse::clickhouse_logging::storage::ClickHouseStorage;
use betterlite_logging::trace_types::SpendLogRecord;
use tokio_util::task::TaskTracker;

pub const CALLBACK_NAME: &str = "ClickHouseSpendLogger";

pub struct SharedStorage<Storage: ClickHouseStorage> {
    shared: Arc<Storage>,
}

impl<Storage: ClickHouseStorage> ClickHouseStorage for SharedStorage<Storage> {
    type Error = Storage::Error;

    fn insert_rows(
        &self,
        table: &str,
        rows: &[SpendLogRecord],
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.shared.insert_rows(table, rows)
    }

    fn ensure_schema(
        &self,
        trace_retention_days: u32,
        spend_log_retention_days: u32,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.shared
            .ensure_schema(trace_retention_days, spend_log_retention_days)
    }
}

pub trait SpendLoggerFactory<Storage: ClickHouseStorage>: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn build(
        &self,
        storage: Arc<Storage>,
    ) -> Result<ClickHouseSpendLogger<SharedStorage<Storage>>, Self::Error>;

    // B4001's projection marks rows from the Lens context injected here, so one shared context paints them all
    fn lens(&self) -> LensAnalysis;
}

pub struct ClickHouseSpendLoggers {
    pub settings: ClickHouseSettings,
    pub tasks: TaskTracker,
    pub lens: LensAnalysis,
}

impl<Storage: ClickHouseStorage> SpendLoggerFactory<Storage> for ClickHouseSpendLoggers {
    type Error = Infallible;

    fn build(
        &self,
        storage: Arc<Storage>,
    ) -> Result<ClickHouseSpendLogger<SharedStorage<Storage>>, Infallible> {
        Ok(ClickHouseSpendLogger {
            batch: ClickHouseBatchLogger::new(
                SharedStorage { shared: storage },
                SPEND_LOGS_TABLE,
                self.settings.clone(),
                self.tasks.clone(),
            ),
        })
    }

    fn lens(&self) -> LensAnalysis {
        self.lens.clone()
    }
}

pub(crate) struct TraceSpendLogger<Storage: ClickHouseStorage> {
    pub(crate) spend: ClickHouseSpendLogger<SharedStorage<Storage>>,
    pub(crate) lens: LensAnalysis,
}

// B8740's event carries the typed payload, so it crosses into B4001's kwargs shape through the payload's own wire form
fn callback_kwargs(event: &LoggerEvent) -> Result<CallbackKwargs, LoggerError> {
    let encoded =
        serde_json::to_value(&*event.payload).map_err(|source| delivery_error(&source))?;
    let payload = serde_json::from_value(encoded).map_err(|source| delivery_error(&source))?;
    Ok(CallbackKwargs {
        standard_logging_object: Some(payload),
        litellm_params: None,
    })
}

fn delivery_error(source: &serde_json::Error) -> LoggerError {
    LoggerError::Delivery {
        callback: CALLBACK_NAME.to_owned(),
        message: source.to_string(),
    }
}

#[derive(Clone, Copy)]
enum Outcome {
    Success,
    Failure,
}

impl<Storage: ClickHouseStorage> TraceSpendLogger<Storage> {
    async fn log(&self, event: &LoggerEvent, outcome: Outcome) -> Result<(), LoggerError> {
        let kwargs = callback_kwargs(event)?;
        let callback = CallbackEvent {
            kwargs: &kwargs,
            redact: false,
            lens: &self.lens,
        };
        match outcome {
            Outcome::Success => self.spend.async_log_success_event(&callback).await,
            Outcome::Failure => self.spend.async_log_failure_event(&callback).await,
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl<Storage: ClickHouseStorage> CallbackLogger for TraceSpendLogger<Storage> {
    fn callback_name(&self) -> &'static str {
        CALLBACK_NAME
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log(&event, Outcome::Success).await
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.log(&event, Outcome::Failure).await
    }
}
