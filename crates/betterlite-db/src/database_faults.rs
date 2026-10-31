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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lineage {
    ConnectionOnly,
    ConnectionAndPrisma,
    PrismaBaseData,
    PrismaOnly,
    Neither,
}

const fn lineage(error_type: ErrorType) -> Lineage {
    match error_type {
        ErrorType::Connect
        | ErrorType::ConnectTimeout
        | ErrorType::Read
        | ErrorType::ReadTimeout
        | ErrorType::Deadline
        | ErrorType::NoDbConnection => Lineage::ConnectionOnly,
        ErrorType::EngineConnection => Lineage::ConnectionAndPrisma,
        ErrorType::Data => Lineage::PrismaBaseData,
        ErrorType::Prisma
        | ErrorType::ClientNotConnected
        | ErrorType::HttpClientClosed
        | ErrorType::UniqueViolation
        | ErrorType::ForeignKeyViolation
        | ErrorType::MissingRequiredValue
        | ErrorType::RawQuery
        | ErrorType::TableNotFound
        | ErrorType::RecordNotFound => Lineage::PrismaOnly,
        ErrorType::Other => Lineage::Neither,
    }
}

#[must_use]
pub const fn is_database_connection_error(error_type: ErrorType) -> bool {
    match lineage(error_type) {
        Lineage::ConnectionOnly | Lineage::ConnectionAndPrisma => true,
        Lineage::PrismaBaseData | Lineage::PrismaOnly | Lineage::Neither => false,
    }
}

#[must_use]
pub const fn is_prisma_error(error_type: ErrorType) -> bool {
    match lineage(error_type) {
        Lineage::ConnectionAndPrisma | Lineage::PrismaBaseData | Lineage::PrismaOnly => true,
        Lineage::ConnectionOnly | Lineage::Neither => false,
    }
}

#[must_use]
pub const fn is_base_data_error(error_type: ErrorType) -> bool {
    use Lineage::{ConnectionAndPrisma, ConnectionOnly, Neither, PrismaBaseData, PrismaOnly};
    match lineage(error_type) {
        PrismaBaseData => true,
        ConnectionOnly | ConnectionAndPrisma | PrismaOnly | Neither => false,
    }
}

#[must_use]
pub const fn is_infrastructure_error(error_type: ErrorType) -> bool {
    if error_type.is_data_type() || matches!(error_type, ErrorType::Deadline) {
        return false;
    }
    is_database_connection_error(error_type) || is_prisma_error(error_type)
}

#[must_use]
pub fn is_error_type_or_subclass(error_type: ErrorType, ancestor: ErrorType) -> bool {
    if error_type == ancestor {
        return true;
    }
    match ancestor {
        ErrorType::Prisma => is_prisma_error(error_type),
        ErrorType::Data => error_type.is_data_type(),
        ErrorType::Connect
        | ErrorType::ConnectTimeout
        | ErrorType::Read
        | ErrorType::ReadTimeout
        | ErrorType::Deadline
        | ErrorType::EngineConnection
        | ErrorType::NoDbConnection
        | ErrorType::UniqueViolation
        | ErrorType::ForeignKeyViolation
        | ErrorType::MissingRequiredValue
        | ErrorType::RawQuery
        | ErrorType::TableNotFound
        | ErrorType::RecordNotFound
        | ErrorType::ClientNotConnected
        | ErrorType::HttpClientClosed
        | ErrorType::Other => false,
    }
}

#[must_use]
pub fn is_database_transport_error(error: &DbError) -> bool {
    match error.error_type {
        ErrorType::Connect
        | ErrorType::ConnectTimeout
        | ErrorType::Read
        | ErrorType::ReadTimeout
        | ErrorType::ClientNotConnected
        | ErrorType::HttpClientClosed
        | ErrorType::NoDbConnection => true,
        ErrorType::Deadline | ErrorType::Other => false,
        ErrorType::EngineConnection
        | ErrorType::Prisma
        | ErrorType::Data
        | ErrorType::UniqueViolation
        | ErrorType::ForeignKeyViolation
        | ErrorType::MissingRequiredValue
        | ErrorType::RawQuery
        | ErrorType::TableNotFound
        | ErrorType::RecordNotFound => contains_any(&error.message, &TRANSPORT_WORDS),
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
    if !is_prisma_error(error.error_type) {
        return false;
    }
    has_deadlock_marker(error)
}

#[must_use]
pub fn postgres_sqlstate(error: &DbError) -> Option<String> {
    if !error.error_type.is_data_type() {
        return None;
    }
    if let Some(code) = &error.meta_code {
        return Some(code.clone());
    }
    let pattern = regex::Regex::new(r#"PostgresError \{ code: "([0-9A-Z]{5})""#).ok()?;
    pattern
        .captures(&error.message)
        .and_then(|captures| captures.get(1))
        .map(|capture| capture.as_str().to_owned())
}

#[must_use]
pub fn is_read_only_transaction_error(error: &DbError) -> bool {
    if !is_prisma_error(error.error_type) {
        return false;
    }
    contains_any(&error.message, &["\"25006\"", "read-only transaction"])
}

#[must_use]
pub fn is_engine_internal_error(error: &DbError) -> bool {
    if is_prisma_error(error.error_type) {
        return false;
    }
    error
        .traceback_modules
        .iter()
        .any(|module| module.starts_with("prisma.engine"))
}

#[must_use]
pub fn is_permanent_database_fault(error: &DbError) -> bool {
    if is_database_connection_error(error.error_type) {
        return false;
    }
    if is_database_transport_error(error) {
        return false;
    }
    is_infrastructure_error(error.error_type)
}

pub mod fixtures;
pub mod native;
pub mod reconnect;
mod tests;
