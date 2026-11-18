use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::database_faults::native::NativeFault;
use betterlite_db::database_faults::native::classify_sqlstate;
use betterlite_db::postgres_store::diagnostics::DriverError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_repository::config::ConfigError;

// LiteLLM reads these out of the message, so a timeout is a 503 and a user cancel is not.
const OUTAGE_MESSAGES: [&str; 13] = [
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
    "cached plan must not change result type",
];

pub(super) fn is_database_outage(error: &ConfigError) -> bool {
    let database_error = match error {
        ConfigError::Database(source) | ConfigError::Table(TableError::Database(source)) => source,
        ConfigError::Table(_) | ConfigError::Json(_) => return false,
    };
    match database_error {
        DbError::NoDbConnected | DbError::Pool { .. } | DbError::Backend { .. } => true,
        DbError::SqlState { code, source, .. } => {
            code.code().starts_with("08")
                || classify_sqlstate(code) == NativeFault::ConnectionUnavailable
                || carries_a_transport_message(source)
        }
        DbError::InvalidRecord { .. } | DbError::CreateNull | DbError::UniqueViolation { .. } => {
            false
        }
    }
}

fn carries_a_transport_message(source: &DriverError) -> bool {
    let Some(error) = source.driver.as_db_error() else {
        return false;
    };
    let message = error.message().to_ascii_lowercase();
    OUTAGE_MESSAGES
        .iter()
        .any(|keyword| message.contains(keyword))
}
