use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
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

use super::error::{KeyReadError, READ_CALL_TYPE};
use super::grace::DeprecatedKeyCache;
use super::ports::{
    Diagnostic, KeyFailureHook, KeyReadDatabase, KeyReadDiagnostics, MetricWallClock, PlanReconnect,
};
use super::request::{COMBINED_VIEW_TABLE, CombinedKeyRequest, KeyQueryType};
use super::retry::{parked_lookup_error, retry_wait, take_parked};
use super::view::{CombinedKeyView, epoch_decimal};

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
    pub async fn read(
        &self,
        request: &CombinedKeyRequest<'_>,
    ) -> Result<Option<CombinedKeyView>, KeyReadError> {
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
    }

    // log_db_metrics wraps each get_data call: it awaits a failure metric and schedules a success metric
    async fn measured_attempt(
        &self,
        request: &CombinedKeyRequest<'_>,
    ) -> Result<Option<CombinedKeyView>, KeyReadError> {
        let parked: Mutex<Option<KeyReadError>> = Mutex::new(None);
        let table = Json::Text(String::from(COMBINED_VIEW_TABLE));
        let call = DbMetricCall {
            function_name: READ_CALL_TYPE,
            parent_otel_span: request.parent_otel_span.clone(),
            table_name: Some(&table),
            second_positional: None,
        };
        let operation = async {
            match self.attempt(request).await {
                Ok(view) => Ok(view),
                Err(KeyReadError::Database(fault)) => Err(DbLookupError::Database(fault)),
                Err(error) => {
                    *parked.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
                    Err(parked_lookup_error())
                }
            }
        };
        let metric_clock = MetricWallClock(self.services.clock);
        let context = DbMetricContext {
            sink: Arc::clone(&self.sink),
            resolver: self.services.resolver,
            clock: &metric_clock,
            tasks: self.services.tasks,
        };
        let outcome = log_db_metrics(operation, &call, context).await;
        match outcome {
            Ok(view) => Ok(view),
            Err(DbMetricError::Lookup(DbLookupError::Database(fault))) => {
                Err(KeyReadError::Database(fault))
            }
            Err(DbMetricError::Lookup(_non_database)) => Err(take_parked(parked)),
            Err(DbMetricError::Metric(failure)) => Err(KeyReadError::Metrics {
                message: failure.to_string(),
            }),
        }
    }

    // the get_data body plus its error boundary: a failed attempt reports before the metric and backoff run
    async fn attempt(
        &self,
        request: &CombinedKeyRequest<'_>,
    ) -> Result<Option<CombinedKeyView>, KeyReadError> {
        let started = self.services.clock.instant();
        let outcome = self.lookup(request).await;
        match outcome {
            Ok(view) => Ok(view),
            Err(error) => {
                self.report_failure(&error, started);
                Err(error)
            }
        }
    }

    async fn lookup(
        &self,
        request: &CombinedKeyRequest<'_>,
    ) -> Result<Option<CombinedKeyView>, KeyReadError> {
        let Some(token) = request.token else {
            return match request.query_type {
                KeyQueryType::FindUnique => Err(KeyReadError::MissingToken),
                KeyQueryType::FindAll => Ok(None),
            };
        };
        let hash = KeyHash::from(hash_token_if_needed(token));
        self.services.diagnostics.emit(Diagnostic::TokenHash);
        if request.query_type != KeyQueryType::FindUnique {
            return Ok(None);
        }
        let row = self.read_row(&hash, request.use_writer).await?;
        if row.is_none()
            && request.check_deprecated
            && let Some(active_token_id) = self.grace_lookup(&hash, request.use_writer).await
        {
            return self.grace_read(request, &active_token_id).await;
        }
        match row {
            Some(fields) => {
                let refreshed = epoch_decimal(self.services.clock.now());
                let view = CombinedKeyView::construct(fields, refreshed)?;
                Ok(Some(view))
            }
            None => Ok(None),
        }
    }

    // the deprecated key recurses once with check_deprecated off, so a grace lookup can never chain
    async fn grace_read(
        &self,
        request: &CombinedKeyRequest<'_>,
        active_token_id: &str,
    ) -> Result<Option<CombinedKeyView>, KeyReadError> {
        let recursive = CombinedKeyRequest {
            token: Some(active_token_id),
            query_type: KeyQueryType::FindUnique,
            check_deprecated: false,
            use_writer: request.use_writer,
            parent_otel_span: request.parent_otel_span.clone(),
        };
        let found = Box::pin(self.read(&recursive)).await?;
        if found.is_some() {
            self.services.diagnostics.emit(Diagnostic::DeprecatedUsed);
        }
        Ok(found)
    }
}
