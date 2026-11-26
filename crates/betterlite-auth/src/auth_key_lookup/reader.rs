use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::combined_key_read::error::KeyReadError;
use betterlite_db::combined_key_read::view::CombinedKeyView;
use betterlite_db::database_failures::deadline::{DbLookupError, LookupRequest, bounded_db_lookup};
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock,
};
use betterlite_db::database_faults::reconnect::DbReconnect;
use betterlite_db::database_faults::{DbError, ErrorType};
use opentelemetry::trace::SpanContext;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::task::TaskTracker;

use super::error::{KeyLookupError, PARKED_FAULT, transport_fault};

// The bounded lookup's name and the reconnect reason LiteLLM passes for a key read.
pub const KEY_LOOKUP: &str = "key";
pub const AUTH_RECONNECT_REASON: &str = "auth_get_key_object_lookup_failure";
pub const RECONNECT_TIMEOUT_SECONDS: f64 = 2.0;
pub const RECONNECT_LOCK_SECONDS: f64 = 0.1;

// B2320's get_data branch, reached by the caller with the supplied hash and the requested table.
#[async_trait]
pub trait KeySource: Send + Sync {
    async fn read_key(
        &self,
        hashed_token: &str,
        use_writer: bool,
        parent_otel_span: Option<SpanContext>,
    ) -> Result<Option<CombinedKeyView>, KeyReadError>;
}

// one deadline bounds the gate, the read, the reconnect and the single retry.
pub struct KeyReader {
    pub source: Arc<dyn KeySource>,
    pub reconnect: Option<Arc<AsyncMutex<Box<dyn DbReconnect + Send>>>>,
    pub gate: Arc<LoopBoundSemaphore>,
    pub tracker: Arc<DbLookupStallTracker<SystemClock>>,
    pub tasks: TaskTracker,
    pub default_deadline_seconds: f64,
}

impl KeyReader {
    pub async fn fetch(
        &self,
        hashed_token: &str,
        check_db_only: bool,
        parent_otel_span: Option<SpanContext>,
    ) -> Result<Option<CombinedKeyView>, KeyLookupError> {
        let parked = Arc::new(Mutex::new(None));
        let call = ReadCall {
            source: Arc::clone(&self.source),
            reconnect: self.reconnect.as_ref().map(Arc::clone),
            gate: Arc::clone(&self.gate),
            parked: Arc::clone(&parked),
            token: hashed_token.to_owned(),
            use_writer: check_db_only,
            parent_otel_span,
        };
        let request = LookupRequest {
            name: KEY_LOOKUP,
            deadline_seconds: None,
            default_deadline_seconds: self.default_deadline_seconds,
            tracker: self.tracker.as_ref(),
        };
        match bounded_db_lookup(read(call), request, &self.tasks).await {
            Ok(found) => Ok(found),
            Err(error) => {
                let failure = parked.lock().unwrap_or_else(PoisonError::into_inner).take();
                Err(failure.map_or_else(|| KeyLookupError::Lookup(error), KeyLookupError::Read))
            }
        }
    }
}

struct ReadCall {
    source: Arc<dyn KeySource>,
    reconnect: Option<Arc<AsyncMutex<Box<dyn DbReconnect + Send>>>>,
    gate: Arc<LoopBoundSemaphore>,
    parked: Arc<Mutex<Option<KeyReadError>>>,
    token: String,
    use_writer: bool,
    parent_otel_span: Option<SpanContext>,
}

async fn read(call: ReadCall) -> Result<Option<CombinedKeyView>, DbLookupError> {
    let _permit = match call.gate.current() {
        Some(gate) => gate.acquire_owned().await.ok(),
        None => None,
    };
    let first = call
        .source
        .read_key(&call.token, call.use_writer, call.parent_otel_span.clone())
        .await;
    let failure = match first {
        Ok(found) => return Ok(found),
        Err(failure) => failure,
    };
    let Some(fault) = transport_fault(&failure) else {
        return Err(park(&call.parked, failure));
    };
    reconnect_after_failure(&call, fault).await?;
    match call
        .source
        .read_key(&call.token, call.use_writer, call.parent_otel_span)
        .await
    {
        Ok(found) => Ok(found),
        Err(retry_failure) => match transport_fault(&retry_failure) {
            Some(retry_fault) => Err(DbLookupError::Database(retry_fault)),
            None => Err(park(&call.parked, retry_failure)),
        },
    }
}

async fn reconnect_after_failure(call: &ReadCall, fault: DbError) -> Result<(), DbLookupError> {
    let Some(reconnect) = call.reconnect.as_ref() else {
        return Err(DbLookupError::Database(fault));
    };
    let outcome = {
        let mut client = reconnect.lock().await;
        let timeout_seconds = client
            .reconnect_timeout_seconds()
            .unwrap_or(RECONNECT_TIMEOUT_SECONDS);
        let lock_timeout_seconds = client
            .reconnect_lock_timeout_seconds()
            .unwrap_or(RECONNECT_LOCK_SECONDS);
        client
            .attempt_db_reconnect(AUTH_RECONNECT_REASON, timeout_seconds, lock_timeout_seconds)
            .await
    };
    match outcome {
        Ok(true) => Ok(()),
        Ok(false) => Err(DbLookupError::Database(fault)),
        Err(error) => Err(DbLookupError::Database(error)),
    }
}

fn park(parked: &Mutex<Option<KeyReadError>>, error: KeyReadError) -> DbLookupError {
    *parked.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
    DbLookupError::Database(DbError {
        error_type: ErrorType::Other,
        message: PARKED_FAULT.to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    })
}
