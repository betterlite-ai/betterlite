use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use secrecy::ExposeSecret as _;
use tokio_util::task::TaskTracker;

use crate::database_failures::deadline::DbLookupError;
use crate::database_failures::metrics::{
    DbMetricCall, DbMetricContext, DbMetricError, DbServiceEventSink, ProxySpanResolver,
    log_db_metrics,
};
use crate::key_hash::KeyHash;
use crate::key_user_writes::hash::hash_token_if_needed;
use crate::key_user_writes::retry::{MAX_TIME, MAX_TRIES, WriteClock};
use crate::migration_ledger::lock::JitterSource;
use crate::tool_spend::RetryPause;

use super::error::{DatabaseFailure, KeyReadError, READ_CALL_TYPE};
use super::grace::DeprecatedKeyCache;
use super::ports::{
    Diagnostic, KeyFailureHook, KeyReadDatabase, KeyReadDiagnostics, MetricWallClock, PlanReconnect,
};
use super::report::KeyFailureReport as _;
use super::request::{COMBINED_VIEW_TABLE, CombinedKeyRequest, KeyQueryType};
use super::retry::{parked_lookup_error, retry_wait, take_parked};
use super::rows::KeyRowRead as _;
use super::view::{CombinedKeyView, epoch_decimal};

// the deprecated key re-enters read, so its future is boxed to keep the Send proof from cycling
pub type ReadFuture<'reader> =
    Pin<Box<dyn Future<Output = Result<Option<CombinedKeyView>, KeyReadError>> + Send + 'reader>>;

pub struct ReadServices<'services> {
    pub clock: &'services dyn WriteClock,
    pub jitter: &'services dyn JitterSource,
    pub pause: &'services dyn RetryPause,
    pub diagnostics: &'services dyn KeyReadDiagnostics,
    pub hook: &'services dyn KeyFailureHook,
    pub resolver: &'services dyn ProxySpanResolver,
    pub tasks: &'services TaskTracker,
}

pub struct KeyReader<'reader, Database, Reconnect, Sink>
where
    Database: KeyReadDatabase,
    Reconnect: PlanReconnect,
    Sink: DbServiceEventSink + 'static,
{
    pub database: &'reader Database,
    pub reconnect: &'reader Reconnect,
    pub sink: Arc<Sink>,
    pub services: ReadServices<'reader>,
    pub cache: Arc<Mutex<DeprecatedKeyCache>>,
}

impl<Database, Reconnect, Sink> KeyReader<'_, Database, Reconnect, Sink>
where
    Database: KeyReadDatabase,
    Reconnect: PlanReconnect,
    Sink: DbServiceEventSink + 'static,
{
    // the backoff decorator around get_data: at most three attempts and ten seconds, jittered waits
    #[must_use]
    pub fn read<'reader>(
        &'reader self,
        request: &'reader CombinedKeyRequest,
    ) -> ReadFuture<'reader> {
        Box::pin(async move {
            let start = self.services.clock.instant();
            let mut tries: u32 = 0;
            loop {
                tries = tries.saturating_add(1);
                let elapsed = self
                    .services
                    .clock
                    .instant()
                    .saturating_duration_since(start);
                let error = match self.measured_attempt(request).await {
                    Ok(view) => return Ok(view),
                    Err(error) => error,
                };
                if tries >= MAX_TRIES || elapsed >= MAX_TIME {
                    return Err(error);
                }
                let wait = retry_wait(self.services.jitter, tries, elapsed);
                self.services
                    .diagnostics
                    .emit(Diagnostic::Backoff { tries });
                self.services.pause.pause(wait).await;
            }
        })
    }

    async fn lookup(
        &self,
        request: &CombinedKeyRequest,
    ) -> Result<Option<CombinedKeyView>, KeyReadError> {
        let Some(token) = request.token.as_ref() else {
            return match request.query_type {
                KeyQueryType::FindUnique => Err(KeyReadError::MissingToken),
                KeyQueryType::FindAll => Ok(None),
            };
        };
        let hash = KeyHash::from(hash_token_if_needed(token.expose_secret()));
        self.services.diagnostics.emit(Diagnostic::TokenHash);
        if request.query_type != KeyQueryType::FindUnique {
            return Ok(None);
        }
        let row = self.read_row(&hash, request.use_writer).await?;
        match row {
            Some(fields) => Ok(Some(CombinedKeyView::construct(
                fields,
                epoch_decimal(self.services.clock.now()),
            )?)),
            None if request.check_deprecated => {
                self.deprecated_view(request, &hash, request.use_writer)
                    .await
            }
            None => Ok(None),
        }
    }

    // the grace row only matters when the combined view missed, and its recursive read never checks again
    async fn deprecated_view(
        &self,
        request: &CombinedKeyRequest,
        hash: &KeyHash,
        use_writer: bool,
    ) -> Result<Option<CombinedKeyView>, KeyReadError> {
        let Some(active_token_id) = self.grace_lookup(hash, use_writer).await else {
            return Ok(None);
        };
        // the deprecated key recurses once with check_deprecated off, so a grace lookup can never chain
        let hop = CombinedKeyRequest {
            token: Some(active_token_id),
            query_type: KeyQueryType::FindUnique,
            check_deprecated: false,
            use_writer: request.use_writer,
            parent_otel_span: request.parent_otel_span.clone(),
        };
        let found = self.read(&hop).await?;
        if found.is_some() {
            self.services.diagnostics.emit(Diagnostic::DeprecatedUsed);
        }
        Ok(found)
    }

    // log_db_metrics wraps each get_data call: it awaits a failure metric and schedules a success metric
    async fn measured_attempt(
        &self,
        query: &CombinedKeyRequest,
    ) -> Result<Option<CombinedKeyView>, KeyReadError> {
        let parked: Mutex<Option<KeyReadError>> = Mutex::new(None);
        let table = Json::Text(String::from(COMBINED_VIEW_TABLE));
        let call = DbMetricCall {
            function_name: READ_CALL_TYPE,
            parent_otel_span: query.parent_otel_span.clone(),
            table_name: Some(&table),
            second_positional: None,
        };
        let operation = self.lookup_operation(query, &parked);
        let metric_clock = MetricWallClock(self.services.clock);
        let context = DbMetricContext {
            sink: Arc::clone(&self.sink),
            resolver: self.services.resolver,
            clock: &metric_clock,
            tasks: self.services.tasks,
        };
        metric_outcome(log_db_metrics(operation, &call, context).await, parked)
    }

    // one attempt records its own failure and hands the database fault to the metric wrapper
    async fn lookup_operation(
        &self,
        request: &CombinedKeyRequest,
        parked: &Mutex<Option<KeyReadError>>,
    ) -> Result<Option<CombinedKeyView>, DbLookupError> {
        let started = self.services.clock.instant();
        let error = match self.lookup(request).await {
            Ok(view) => return Ok(view),
            Err(error) => error,
        };
        self.report_failure(&error, started);
        match error {
            KeyReadError::Database(failure) => {
                Err(DbLookupError::Database(failure.into_db_error()))
            }
            other @ (KeyReadError::MissingToken
            | KeyReadError::View(_)
            | KeyReadError::Reconnect(_)
            | KeyReadError::Metrics { .. }) => {
                *parked.lock().unwrap_or_else(PoisonError::into_inner) = Some(other);
                Err(parked_lookup_error())
            }
        }
    }
}

// the metric wrapper hands back the fault, the parked non-database error, or its own failure
fn metric_outcome<Failure>(
    outcome: Result<Option<CombinedKeyView>, DbMetricError<Failure>>,
    parked: Mutex<Option<KeyReadError>>,
) -> Result<Option<CombinedKeyView>, KeyReadError>
where
    Failure: Error + 'static,
{
    match outcome {
        Ok(view) => Ok(view),
        Err(DbMetricError::Lookup(DbLookupError::Database(fault))) => {
            Err(KeyReadError::Database(DatabaseFailure::Reported(fault)))
        }
        Err(DbMetricError::Lookup(_non_database)) => Err(take_parked(parked)),
        Err(DbMetricError::Metric(failure)) => Err(KeyReadError::Metrics {
            message: failure.to_string(),
        }),
    }
}
