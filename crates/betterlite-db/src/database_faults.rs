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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl DbError {
    #[must_use]
    pub const fn new(error_type: ErrorType) -> Self {
        if matches!(error_type, ErrorType::Other) {
            return Self {
                error_type,
                message: String::new(),
                code: None,
                meta_code: None,
                traceback_modules: Vec::new(),
            };
        }
        Self {
            error_type,
            message: String::new(),
            code: None,
            meta_code: None,
            traceback_modules: Vec::new(),
        }
    }
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
        | ErrorType::Deadline
        | ErrorType::EngineConnection
        | ErrorType::NoDbConnection
        | ErrorType::ClientNotConnected
        | ErrorType::HttpClientClosed => true,
        ErrorType::Data
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
    if matches!(
        error.error_type,
        ErrorType::Connection
            | ErrorType::Deadline
            | ErrorType::EngineConnection
            | ErrorType::NoDbConnection
    ) {
        return true;
    }
    false
}

#[must_use]
pub fn is_prisma_data_error(error: &DbError) -> bool {
    if error.error_type == ErrorType::Data {
        return true;
    }
    false
}

#[must_use]
pub fn is_database_infrastructure(error: &DbError) -> bool {
    if error.error_type.is_data_type() {
        return false;
    }
    is_database_connection_error(error) || error.error_type == ErrorType::Prisma
}

#[must_use]
pub fn is_database_transport_error(error: &DbError) -> bool {
    if is_transport_type(error.error_type) {
        return true;
    }
    error.error_type == ErrorType::Prisma && contains_any(&error.message, &TRANSPORT_WORDS)
}

#[must_use]
pub fn is_prisma_error(error: &DbError) -> bool {
    if error.error_type.is_data_type() {
        return true;
    }
    error.error_type == ErrorType::Prisma
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
    if !is_prisma_data_error(error) {
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
    match client
        .attempt_db_reconnect(
            policy.reason,
            policy.timeout_seconds.unwrap_or(2.0_f64),
            policy.lock_timeout_seconds.unwrap_or(0.1_f64),
        )
        .await
    {
        Ok(true) => read().await,
        Ok(false) => Err(error),
        Err(reconnect_error) => Err(DbError {
            message: format!(
                "{}; reconnect failed: {}",
                error.message, reconnect_error.message
            ),
            ..error
        }),
    }
}

pub mod fixtures;
mod tests;
