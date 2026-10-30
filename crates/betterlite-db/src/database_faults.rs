use serde::{Deserialize, Serialize};

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
    Connect,
    ConnectTimeout,
    Read,
    ReadTimeout,
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
            Self::Connect
            | Self::ConnectTimeout
            | Self::Read
            | Self::ReadTimeout
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

fn contains_any(message: &str, phrases: &[&str]) -> bool {
    let lower = message.to_ascii_lowercase();
    phrases.iter().any(|phrase| lower.contains(phrase))
}

#[must_use]
pub const fn is_database_connection_error(error: &DbError) -> bool {
    match error.error_type {
        ErrorType::Connect
        | ErrorType::ConnectTimeout
        | ErrorType::Read
        | ErrorType::ReadTimeout
        | ErrorType::Deadline
        | ErrorType::EngineConnection
        | ErrorType::NoDbConnection => true,
        ErrorType::Data
        | ErrorType::UniqueViolation
        | ErrorType::ForeignKeyViolation
        | ErrorType::MissingRequiredValue
        | ErrorType::RawQuery
        | ErrorType::TableNotFound
        | ErrorType::RecordNotFound
        | ErrorType::Prisma
        | ErrorType::ClientNotConnected
        | ErrorType::HttpClientClosed
        | ErrorType::Other => false,
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
pub fn is_transport_error(error: &DbError) -> bool {
    if is_prisma_error(error) {
        return matches!(
            error.error_type,
            ErrorType::ClientNotConnected | ErrorType::HttpClientClosed
        ) || contains_any(&error.message, &TRANSPORT_WORDS);
    }
    is_database_connection_error(error) && error.error_type != ErrorType::Deadline
}

#[must_use]
pub const fn is_prisma_error(error: &DbError) -> bool {
    match error.error_type {
        ErrorType::Prisma
        | ErrorType::EngineConnection
        | ErrorType::ClientNotConnected
        | ErrorType::HttpClientClosed
        | ErrorType::Data
        | ErrorType::UniqueViolation
        | ErrorType::ForeignKeyViolation
        | ErrorType::MissingRequiredValue
        | ErrorType::RawQuery
        | ErrorType::TableNotFound
        | ErrorType::RecordNotFound => true,
        ErrorType::Connect
        | ErrorType::ConnectTimeout
        | ErrorType::Read
        | ErrorType::ReadTimeout
        | ErrorType::Deadline
        | ErrorType::NoDbConnection
        | ErrorType::Other => false,
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
    error.message.match_indices(marker).find_map(|(offset, _)| {
        let start = offset.checked_add(marker.len())?;
        let end = start.checked_add(5)?;
        if error.message.get(end..end.checked_add(1)?)? != "\"" {
            return None;
        }
        let value = error.message.get(start..end)?;
        value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
            .then(|| value.to_owned())
    })
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
    !is_database_connection_error(error) && !is_transport_error(error)
}

pub mod fixtures;
pub mod reconnect;
mod tests;
