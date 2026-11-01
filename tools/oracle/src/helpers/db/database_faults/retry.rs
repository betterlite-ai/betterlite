use super::capture::{Capture, Effect};
use betterlite_db::database_faults::reconnect::{
    DbReconnect, RetryPolicy, Timeout, call_with_db_reconnect_retry,
};
use betterlite_db::database_faults::{DbError, ErrorType};
use futures_util::FutureExt as _;
use serde::{Deserialize, Serialize};
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::SubscriberExt as _;

#[derive(Deserialize)]
pub(crate) struct Script {
    name: String,
    reads: Vec<Result<i32, DbError>>,
    reconnect: Option<Result<bool, DbError>>,
    reason: String,
    retry_safe_error_types: Option<Vec<ErrorType>>,
    timeout_seconds: Option<Timeout>,
    lock_timeout_seconds: Option<Timeout>,
    client_timeout_seconds: Option<Timeout>,
    client_lock_timeout_seconds: Option<Timeout>,
}

#[derive(Serialize)]
pub(super) struct Report {
    name: String,
    result: Result<i32, &'static str>,
    cause: Option<&'static str>,
    effects: Vec<Effect>,
}

struct Client<'script> {
    script: &'script Script,
    capture: Capture,
}
impl DbReconnect for Client<'_> {
    fn reconnect_timeout_seconds(&self) -> Option<f64> {
        self.script
            .client_timeout_seconds
            .as_ref()
            .map(|value| value.coerce(2.0))
    }
    fn reconnect_lock_timeout_seconds(&self) -> Option<f64> {
        self.script
            .client_lock_timeout_seconds
            .as_ref()
            .map(|value| value.coerce(0.1))
    }
    fn attempt_db_reconnect<'client>(
        &'client mut self,
        reason: &'client str,
        timeout_seconds: f64,
        lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'client>> {
        self.capture
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Effect::Reconnect {
                reason: reason.to_owned(),
                timeout_seconds,
                lock_timeout_seconds,
            });
        Box::pin(ready(self.script.reconnect.clone().unwrap_or(Ok(false))))
    }
}

pub(super) fn run(script: &Script) -> Result<Report, Box<crate::helpers::HelperError>> {
    let capture = Capture(Arc::new(Mutex::new(Vec::new())));
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let outcome = tracing::subscriber::with_default(subscriber, || execute(script, &capture))
        .ok_or_else(|| {
            Box::new(crate::helpers::HelperError {
                class: "ScriptPending".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "script unexpectedly suspended".to_owned(),
            })
        })?;
    let cause = outcome
        .as_ref()
        .err()
        .and_then(|error| error.cause.as_ref())
        .map(|error| class(error.error_type));
    let effects = capture
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    Ok(Report {
        name: script.name.clone(),
        result: outcome.map_err(|error| class(error.error_type)),
        cause,
        effects,
    })
}

fn execute(script: &Script, capture: &Capture) -> Option<Result<i32, DbError>> {
    let mut client = Client {
        script,
        capture: capture.clone(),
    };
    let mut reads = script.reads.iter();
    let policy = RetryPolicy {
        reason: &script.reason,
        retry_safe_error_types: script.retry_safe_error_types.as_deref(),
        timeout_seconds: script
            .timeout_seconds
            .as_ref()
            .map(|value| value.coerce(2.0)),
        lock_timeout_seconds: script
            .lock_timeout_seconds
            .as_ref()
            .map(|value| value.coerce(0.1)),
    };
    call_with_db_reconnect_retry(
        script.reconnect.as_ref().map(|_| &mut client),
        || {
            capture
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(Effect::Read);
            ready(reads.next().cloned().unwrap_or_else(|| {
                Err(DbError {
                    error_type: ErrorType::Other,
                    message: "script exhausted".to_owned(),
                    code: None,
                    meta_code: None,
                    traceback_modules: Vec::new(),
                    cause: None,
                })
            }))
        },
        policy,
    )
    .now_or_never()
}

const fn class(kind: ErrorType) -> &'static str {
    match kind {
        ErrorType::Connect => "ConnectError",
        ErrorType::ConnectTimeout => "ConnectTimeout",
        ErrorType::Read => "ReadError",
        ErrorType::ReadTimeout => "ReadTimeout",
        ErrorType::Deadline => "DBLookupDeadlineExceeded",
        ErrorType::EngineConnection => "EngineConnectionError",
        ErrorType::NoDbConnection => "ProxyException",
        ErrorType::Data => "DataError",
        ErrorType::UniqueViolation => "UniqueViolationError",
        ErrorType::ForeignKeyViolation => "ForeignKeyViolationError",
        ErrorType::MissingRequiredValue => "MissingRequiredValueError",
        ErrorType::RawQuery => "RawQueryError",
        ErrorType::TableNotFound => "TableNotFoundError",
        ErrorType::RecordNotFound => "RecordNotFoundError",
        ErrorType::Prisma => "PrismaError",
        ErrorType::ClientNotConnected => "ClientNotConnectedError",
        ErrorType::HttpClientClosed => "HTTPClientClosedError",
        ErrorType::Other => "Exception",
    }
}
