use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use opentelemetry::trace::SpanContext;
use serde::{Deserialize, Serialize};
use tokio_util::task::TaskTracker;

use crate::database_faults::{self, DbError, ErrorType};

use super::deadline::DbLookupError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbService {
    Postgres,
    BatchWriteToDb,
}

impl DbService {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::BatchWriteToDb => "batch_write_to_db",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbEventMetadata {
    pub table_name: String,
}

#[must_use]
pub fn safe_db_event_metadata(table_name: Option<&Json>) -> Option<DbEventMetadata> {
    match table_name? {
        Json::Text(name) => Some(DbEventMetadata {
            table_name: name.clone(),
        }),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

#[derive(Debug, Clone)]
pub struct DbServiceEvent {
    pub service: DbService,
    pub call_type: String,
    pub parent_otel_span: Option<SpanContext>,
    pub duration_seconds: f64,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub event_metadata: Option<DbEventMetadata>,
}

#[derive(Debug, Clone)]
pub struct DbServiceFailureEvent {
    pub event: DbServiceEvent,
    pub error: DbError,
}

pub type DbMetricFuture<'sink, Failure> =
    Pin<Box<dyn Future<Output = Result<(), Failure>> + Send + 'sink>>;

pub trait DbServiceEventSink: Send + Sync {
    type Failure: Error + Send + Sync + 'static;

    fn success(&self, event: DbServiceEvent) -> DbMetricFuture<'_, Self::Failure>;

    fn failure(&self, event: DbServiceFailureEvent) -> DbMetricFuture<'_, Self::Failure>;
}

pub trait ProxySpanResolver: Send + Sync {
    fn parent_span(&self, passed_kwargs: &IndexMap<String, Json>) -> Option<SpanContext>;
}

pub trait WallClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemWallClock;

impl WallClock for SystemWallClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub struct DbMetricCall<'call> {
    pub function_name: &'call str,
    pub parent_otel_span: Option<SpanContext>,
    pub table_name: Option<&'call Json>,
    pub second_positional: Option<&'call Json>,
}

pub struct DbMetricContext<'context, Sink>
where
    Sink: DbServiceEventSink,
{
    pub sink: Arc<Sink>,
    pub resolver: &'context dyn ProxySpanResolver,
    pub clock: &'context dyn WallClock,
    pub tasks: &'context TaskTracker,
}

#[derive(Debug, thiserror::Error)]
pub enum DbMetricError<Failure>
where
    Failure: Error + 'static,
{
    #[error(transparent)]
    Lookup(#[from] DbLookupError),
    #[error(transparent)]
    Metric(Failure),
}

pub async fn log_db_metrics<Operation, Output, Sink>(
    operation: Operation,
    call: &DbMetricCall<'_>,
    context: DbMetricContext<'_, Sink>,
) -> Result<Output, DbMetricError<Sink::Failure>>
where
    Operation: Future<Output = Result<Output, DbLookupError>> + Send,
    Sink: DbServiceEventSink + 'static,
{
    let DbMetricContext {
        sink,
        resolver,
        clock,
        tasks,
    } = context;
    let start_time = clock.now();
    match operation.await {
        Ok(output) => {
            let times = event_times(clock, start_time);
            schedule_success(
                success_event(call, times, resolver),
                Arc::clone(&sink),
                tasks,
            );
            Ok(output)
        }
        Err(error) => {
            let times = event_times(clock, start_time);
            if let Some(failure) = db_service_failure(&error) {
                let event = failure_event(call, times, failure.clone());
                sink.failure(event).await.map_err(DbMetricError::Metric)?;
            }
            Err(DbMetricError::Lookup(error))
        }
    }
}

fn schedule_success<Sink>(event: Option<DbServiceEvent>, sink: Arc<Sink>, tasks: &TaskTracker)
where
    Sink: DbServiceEventSink + 'static,
{
    let Some(scheduled) = event else {
        return;
    };
    drop(tasks.spawn(async move {
        if let Err(failure) = sink.success(scheduled).await {
            tracing::warn!(%failure, "the database success metric failed");
        }
    }));
}

fn db_service_failure(error: &DbLookupError) -> Option<&DbError> {
    let DbLookupError::Database(db_error) = error else {
        return None;
    };
    let related = database_faults::is_prisma_error(db_error.error_type)
        || matches!(
            db_error.error_type,
            ErrorType::Connect | ErrorType::ConnectTimeout | ErrorType::ReadTimeout
        );
    related.then_some(db_error)
}

fn success_event(
    call: &DbMetricCall<'_>,
    times: EventTimes,
    resolver: &dyn ProxySpanResolver,
) -> Option<DbServiceEvent> {
    if call.function_name.contains("PROXY") {
        proxy_success(call, times, resolver)
    } else {
        Some(db_event(
            call,
            times,
            call.parent_otel_span.clone(),
            elapsed_seconds(times),
            safe_db_event_metadata(call.table_name),
        ))
    }
}

fn proxy_success(
    call: &DbMetricCall<'_>,
    times: EventTimes,
    resolver: &dyn ProxySpanResolver,
) -> Option<DbServiceEvent> {
    let Json::Object(passed_kwargs) = call.second_positional? else {
        return None;
    };
    let parent_otel_span = resolver.parent_span(passed_kwargs)?;
    let mut event = db_event(call, times, Some(parent_otel_span), 0.0, None);
    event.service = DbService::BatchWriteToDb;
    Some(event)
}

fn failure_event(
    call: &DbMetricCall<'_>,
    times: EventTimes,
    error: DbError,
) -> DbServiceFailureEvent {
    DbServiceFailureEvent {
        event: db_event(
            call,
            times,
            call.parent_otel_span.clone(),
            elapsed_seconds(times),
            safe_db_event_metadata(call.table_name),
        ),
        error,
    }
}

fn db_event(
    call: &DbMetricCall<'_>,
    times: EventTimes,
    parent_otel_span: Option<SpanContext>,
    duration_seconds: f64,
    event_metadata: Option<DbEventMetadata>,
) -> DbServiceEvent {
    DbServiceEvent {
        service: DbService::Postgres,
        call_type: call.function_name.to_owned(),
        parent_otel_span,
        duration_seconds,
        start_time: times.start,
        end_time: times.end,
        event_metadata,
    }
}

#[derive(Debug, Clone, Copy)]
struct EventTimes {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

fn event_times(clock: &dyn WallClock, start: DateTime<Utc>) -> EventTimes {
    EventTimes {
        start,
        end: clock.now(),
    }
}

fn elapsed_seconds(times: EventTimes) -> f64 {
    let micros = times
        .end
        .signed_duration_since(times.start)
        .num_microseconds()
        .unwrap_or_default();
    let magnitude = Duration::from_micros(micros.unsigned_abs()).as_secs_f64();
    let sign = if micros.is_negative() {
        -1.0_f64
    } else {
        1.0_f64
    };
    magnitude.copysign(sign)
}
