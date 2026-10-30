use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

const TRANSPORT_WORDS: [&str; 12] = [
    "can't reach database server",
    "cannot reach database server",
    "can't connect",
    "cannot connect",
    "connection error",
    "connection closed",
    "timed out",
    "timeout",
    "connection refused",
    "network is unreachable",
    "no route to host",
    "broken pipe",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    Connection,
    Deadline,
    EngineConnection,
    NoDbConnection,
    Data,
    UniqueViolation,
    ForeignKeyViolation,
    MissingRequiredValue,
    RawQuery,
    TableNotFound,
    RecordNotFound,
    Prisma,
    ClientNotConnected,
    HttpClientClosed,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct DbError {
    pub error_type: ErrorType,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub meta_code: Option<String>,
    #[serde(default)]
    pub traceback_modules: Vec<String>,
    #[source]
    #[serde(skip)]
    pub cause: Option<Box<Self>>,
}

impl ErrorType {
    const fn is_data_type(self) -> bool {
        match self {
            Self::Data
            | Self::UniqueViolation
            | Self::ForeignKeyViolation
            | Self::MissingRequiredValue
            | Self::RawQuery
            | Self::TableNotFound
            | Self::RecordNotFound => true,
            Self::Connection
            | Self::Deadline
            | Self::EngineConnection
            | Self::NoDbConnection
            | Self::Prisma
            | Self::ClientNotConnected
            | Self::HttpClientClosed
            | Self::Other => false,
        }
    }
}

const fn is_transport_type(error_type: ErrorType) -> bool {
    match error_type {
        ErrorType::Connection
        | ErrorType::NoDbConnection
        | ErrorType::ClientNotConnected
        | ErrorType::HttpClientClosed => true,
        ErrorType::Deadline
        | ErrorType::EngineConnection
        | ErrorType::Data
        | ErrorType::UniqueViolation
        | ErrorType::ForeignKeyViolation
        | ErrorType::MissingRequiredValue
        | ErrorType::RawQuery
        | ErrorType::TableNotFound
        | ErrorType::RecordNotFound
        | ErrorType::Prisma
        | ErrorType::Other => false,
    }
}

fn contains_any(message: &str, phrases: &[&str]) -> bool {
    let lower = message.to_ascii_lowercase();
    phrases.iter().any(|phrase| lower.contains(phrase))
}

#[must_use]
pub const fn is_database_connection_error(error: &DbError) -> bool {
    match error.error_type {
        ErrorType::Connection
        | ErrorType::Deadline
        | ErrorType::EngineConnection
        | ErrorType::NoDbConnection => true,
        _ => false,
    }
}

#[must_use]
pub fn is_prisma_data_error(error: &DbError) -> bool {
    match error.error_type {
        ErrorType::Data => true,
        _ => false,
    }
}

#[must_use]
pub const fn is_database_infrastructure(error: &DbError) -> bool {
    if error.error_type.is_data_type() {
        return false;
    }
    !matches!(error.error_type, ErrorType::Deadline | ErrorType::Other)
}

#[must_use]
pub fn is_database_transport_error(error: &DbError) -> bool {
    if is_transport_type(error.error_type) {
        return true;
    }
    matches!(
        error.error_type,
        ErrorType::Prisma | ErrorType::EngineConnection
    ) && contains_any(&error.message, &TRANSPORT_WORDS)
}

#[must_use]
pub const fn is_prisma_error(error: &DbError) -> bool {
    match error.error_type {
        ErrorType::Prisma
        | ErrorType::EngineConnection
        | ErrorType::ClientNotConnected
        | ErrorType::HttpClientClosed => true,
        _ => error.error_type.is_data_type(),
    }
}

#[must_use]
fn has_deadlock_marker(error: &DbError) -> bool {
    if error.code.as_deref() == Some("P2034") {
        return true;
    }
    contains_any(
        &error.message,
        &["deadlock detected", "40p01", "write conflict or a deadlock"],
    )
}

#[must_use]
pub fn is_deadlock_error(error: &DbError) -> bool {
    if is_prisma_error(error) {
        return has_deadlock_marker(error);
    }
    false
}

#[must_use]
pub fn postgres_sqlstate(error: &DbError) -> Option<String> {
    if !error.error_type.is_data_type() {
        return None;
    }
    if let Some(code) = &error.meta_code {
        return Some(code.clone());
    }
    let marker = "PostgresError { code: \"";
    let start = error.message.find(marker)?.checked_add(marker.len())?;
    let value = error.message.get(start..start.checked_add(5)?)?;
    value
        .chars()
        .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
        .then(|| value.to_owned())
}

#[must_use]
pub fn is_read_only_transaction_error(error: &DbError) -> bool {
    if !is_prisma_error(error) {
        return false;
    }
    contains_any(&error.message, &["\"25006\"", "read-only transaction"])
}

#[must_use]
pub fn is_prisma_engine_internal(error: &DbError) -> bool {
    if is_prisma_error(error) {
        return false;
    }
    error
        .traceback_modules
        .iter()
        .any(|module| module.starts_with("prisma.engine"))
}

#[must_use]
pub fn is_permanent_database_fault(error: &DbError) -> bool {
    if !is_database_infrastructure(error) {
        return false;
    }
    !is_database_connection_error(error) && !is_database_transport_error(error)
}

pub trait DbReconnect {
    fn reconnect_timeout_seconds(&self) -> Option<f64>;

    fn reconnect_lock_timeout_seconds(&self) -> Option<f64>;

    fn attempt_db_reconnect<'client>(
        &'client mut self,
        reason: &'client str,
        timeout_seconds: f64,
        lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'client>>;
}

pub struct RetryPolicy<'reason> {
    pub reason: &'reason str,
    pub retry_safe_error_types: Option<&'reason [ErrorType]>,
    pub timeout_seconds: Option<f64>,
    pub lock_timeout_seconds: Option<f64>,
}

fn retry_timeouts<Client>(client: &Client, policy: &RetryPolicy<'_>) -> (f64, f64)
where
    Client: DbReconnect,
{
    let timeout_seconds = policy
        .timeout_seconds
        .or_else(|| client.reconnect_timeout_seconds())
        .unwrap_or(2.0_f64);
    let lock_timeout_seconds = policy
        .lock_timeout_seconds
        .or_else(|| client.reconnect_lock_timeout_seconds())
        .unwrap_or(0.1_f64);
    (timeout_seconds, lock_timeout_seconds)
}

pub async fn call_with_db_reconnect_retry<T, Read, ReadFuture, Client>(
    client: &mut Client,
    mut read: Read,
    policy: RetryPolicy<'_>,
) -> Result<T, DbError>
where
    Read: FnMut() -> ReadFuture + Send,
    ReadFuture: Future<Output = Result<T, DbError>> + Send,
    Client: DbReconnect + Send,
    T: Send,
{
    let first = read().await;
    let error = match first {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    if !is_database_transport_error(&error)
        || policy
            .retry_safe_error_types
            .is_some_and(|allowed| !allowed.contains(&error.error_type))
    {
        return Err(error);
    }
    let (timeout_seconds, lock_timeout_seconds) = retry_timeouts(client, &policy);
    tracing::warn!(
        reason = policy.reason,
        error = %error,
        "DB transport error on read; attempting reconnect-and-retry"
    );
    match client
        .attempt_db_reconnect(policy.reason, timeout_seconds, lock_timeout_seconds)
        .await
    {
        Ok(true) => read().await,
        Ok(false) => Err(error),
        Err(reconnect_error) => {
            tracing::warn!(
                reason = policy.reason,
                reconnect_error = %reconnect_error,
                "DB reconnect attempt raised; preserving original transport error"
            );
            Err(DbError {
                cause: Some(Box::new(reconnect_error)),
                ..error
            })
        }
    }
}

pub mod fixtures;
mod tests;
