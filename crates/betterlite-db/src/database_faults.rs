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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorClasses {
    pub connection: bool,
    pub infrastructure: bool,
    pub prisma: bool,
}

#[must_use]
pub const fn classify(error_type: ErrorType) -> ErrorClasses {
    match error_type {
        ErrorType::Connect
        | ErrorType::ConnectTimeout
        | ErrorType::Read
        | ErrorType::ReadTimeout
        | ErrorType::NoDbConnection => ErrorClasses {
            connection: true,
            infrastructure: true,
            prisma: false,
        },
        ErrorType::Deadline => ErrorClasses {
            connection: true,
            infrastructure: false,
            prisma: false,
        },
        ErrorType::EngineConnection => ErrorClasses {
            connection: true,
            infrastructure: true,
            prisma: true,
        },
        ErrorType::Data
        | ErrorType::UniqueViolation
        | ErrorType::ForeignKeyViolation
        | ErrorType::MissingRequiredValue
        | ErrorType::RawQuery
        | ErrorType::TableNotFound
        | ErrorType::RecordNotFound => ErrorClasses {
            connection: false,
            infrastructure: false,
            prisma: true,
        },
        ErrorType::Prisma | ErrorType::ClientNotConnected | ErrorType::HttpClientClosed => {
            ErrorClasses {
                connection: false,
                infrastructure: true,
                prisma: true,
            }
        }
        ErrorType::Other => ErrorClasses {
            connection: false,
            infrastructure: false,
            prisma: false,
        },
    }
}

#[must_use]
pub fn is_error_type_or_subclass(error_type: ErrorType, ancestor: ErrorType) -> bool {
    if error_type == ancestor {
        return true;
    }
    match ancestor {
        ErrorType::Prisma => classify(error_type).prisma,
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
pub fn is_transport_error(error: &DbError) -> bool {
    let classes = classify(error.error_type);
    if classes.prisma {
        return matches!(
            error.error_type,
            ErrorType::ClientNotConnected | ErrorType::HttpClientClosed
        ) || contains_any(&error.message, &TRANSPORT_WORDS);
    }
    classes.connection && error.error_type != ErrorType::Deadline
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
    if classify(error.error_type).prisma {
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
    let pattern = regex::Regex::new(r#"PostgresError \{ code: "([0-9A-Z]{5})""#).ok()?;
    pattern
        .captures(&error.message)
        .and_then(|captures| captures.get(1))
        .map(|capture| capture.as_str().to_owned())
}

#[must_use]
pub fn is_read_only_transaction_error(error: &DbError) -> bool {
    if !classify(error.error_type).prisma {
        return false;
    }
    contains_any(&error.message, &["\"25006\"", "read-only transaction"])
}

#[must_use]
pub fn is_prisma_engine_internal(error: &DbError) -> bool {
    if classify(error.error_type).prisma {
        return false;
    }
    error
        .traceback_modules
        .iter()
        .any(|module| module.starts_with("prisma.engine"))
}

#[must_use]
pub fn is_permanent_database_fault(error: &DbError) -> bool {
    let classes = classify(error.error_type);
    if !classes.infrastructure {
        return false;
    }
    !classes.connection && !is_transport_error(error)
}

pub mod fixtures;
pub mod native;
pub mod reconnect;
mod tests;
