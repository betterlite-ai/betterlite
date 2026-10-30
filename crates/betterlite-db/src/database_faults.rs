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
    pub fn new(error_type: ErrorType) -> Self {
        let value = Self {
            error_type,
            message: String::new(),
            code: None,
            meta_code: None,
            traceback_modules: Vec::new(),
        };
        value
    }
}
pub fn is_database_connection_error(error: &DbError) -> bool {
    let result = matches!(
        error.error_type,
        ErrorType::Connection
            | ErrorType::Deadline
            | ErrorType::EngineConnection
            | ErrorType::NoDbConnection
    );
    result
}
pub fn is_prisma_data_error(error: &DbError) -> bool {
    let result = error.error_type == ErrorType::Data;
    result
}
pub fn is_database_infrastructure(error: &DbError) -> bool {
    if matches!(
        error.error_type,
        ErrorType::Data
            | ErrorType::UniqueViolation
            | ErrorType::ForeignKeyViolation
            | ErrorType::MissingRequiredValue
            | ErrorType::RawQuery
            | ErrorType::TableNotFound
            | ErrorType::RecordNotFound
    ) {
        return false;
    }
    let result = matches!(
        error.error_type,
        ErrorType::Connection
            | ErrorType::Deadline
            | ErrorType::EngineConnection
            | ErrorType::NoDbConnection
            | ErrorType::Prisma
    );
    result
}
pub fn is_database_transport_error(error: &DbError) -> bool {
    if matches!(
        error.error_type,
        ErrorType::Connection
            | ErrorType::Deadline
            | ErrorType::EngineConnection
            | ErrorType::NoDbConnection
            | ErrorType::ClientNotConnected
            | ErrorType::HttpClientClosed
    ) {
        return true;
    }
    error.error_type == ErrorType::Prisma
        && TRANSPORT_WORDS
            .iter()
            .any(|word| error.message.to_ascii_lowercase().contains(word))
}
pub fn is_prisma_error(error: &DbError) -> bool {
    let result = matches!(
        error.error_type,
        ErrorType::Data
            | ErrorType::UniqueViolation
            | ErrorType::ForeignKeyViolation
            | ErrorType::MissingRequiredValue
            | ErrorType::RawQuery
            | ErrorType::TableNotFound
            | ErrorType::RecordNotFound
            | ErrorType::Prisma
    );
    result
}
pub fn is_deadlock_error(error: &DbError) -> bool {
    let result = is_prisma_error(error)
        && (error.code.as_deref() == Some("P2034")
            || ["deadlock detected", "40p01", "write conflict or a deadlock"]
                .iter()
                .any(|word| error.message.to_ascii_lowercase().contains(word)));
    result
}
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
pub fn is_read_only_transaction_error(error: &DbError) -> bool {
    let result = is_prisma_error(error)
        && (error.message.to_ascii_lowercase().contains("\"25006\"")
            || error
                .message
                .to_ascii_lowercase()
                .contains("read-only transaction"));
    result
}
pub fn is_prisma_engine_internal(error: &DbError) -> bool {
    let result = !is_prisma_error(error)
        && error
            .traceback_modules
            .iter()
            .any(|module| module.starts_with("prisma.engine"));
    result
}
pub fn is_permanent_database_fault(error: &DbError) -> bool {
    let result = is_database_infrastructure(error)
        && !is_database_connection_error(error)
        && !is_database_transport_error(error);
    result
}
pub trait DbReconnect {
    fn attempt_db_reconnect<'a>(
        &'a mut self,
        reason: &'a str,
        timeout_seconds: f64,
        lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + 'a>>;
}
pub async fn call_with_db_reconnect_retry<T, Read, ReadFuture, Client>(
    client: &mut Client,
    mut read: Read,
    reason: &str,
    retry_safe_error_types: Option<&[ErrorType]>,
    timeout_seconds: Option<f64>,
    lock_timeout_seconds: Option<f64>,
) -> Result<T, DbError>
where
    Read: FnMut() -> ReadFuture,
    ReadFuture: Future<Output = Result<T, DbError>>,
    Client: DbReconnect,
{
    let first = read().await;
    let error = match first {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    if !is_database_transport_error(&error)
        || retry_safe_error_types.is_some_and(|allowed| !allowed.contains(&error.error_type))
    {
        return Err(error);
    }
    match client
        .attempt_db_reconnect(
            reason,
            timeout_seconds.unwrap_or(2.0),
            lock_timeout_seconds.unwrap_or(0.1),
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
