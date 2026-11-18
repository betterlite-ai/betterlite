use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::database_faults::native::NativeFault;
use betterlite_db::database_faults::native::classify_sqlstate;
use betterlite_db::postgres_store::diagnostics::DriverError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_repository::config::ConfigError;

// LiteLLM's transport keywords match the message of these timeouts, so its answer is 503
const TIMEOUT_STATES: [&str; 3] = ["25P03", "55P03", "57014"];
const STALE_PLAN: &str = "cached plan must not change result type";

pub(super) fn is_database_outage(error: &ConfigError) -> bool {
    let source = match error {
        ConfigError::Database(source) => source,
        ConfigError::Table(TableError::Database(source)) => source,
        ConfigError::Table(_) | ConfigError::Json(_) => return false,
    };
    match source {
        DbError::NoDbConnected | DbError::Pool { .. } | DbError::Backend { .. } => true,
        DbError::SqlState { code, source, .. } => {
            code.code().starts_with("08")
                || classify_sqlstate(code) == NativeFault::ConnectionUnavailable
                || TIMEOUT_STATES.contains(&code.code())
                || is_stale_plan(source)
        }
        DbError::InvalidRecord { .. } | DbError::CreateNull | DbError::UniqueViolation { .. } => {
            false
        }
    }
}

fn is_stale_plan(source: &DriverError) -> bool {
    source
        .driver
        .as_db_error()
        .is_some_and(|error| error.message().to_ascii_lowercase().contains(STALE_PLAN))
}
