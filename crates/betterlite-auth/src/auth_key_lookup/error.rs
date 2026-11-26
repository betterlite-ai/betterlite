use std::collections::BTreeMap;
use std::error::Error as _;
use std::io::Error as IoError;

use betterlite_db::combined_key_read::error::{DatabaseFailure, KeyReadError};
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::native::{NativeFault, classify_sqlstate};
use betterlite_db::database_faults::{DbError, ErrorType, is_database_transport_error};
use betterlite_db::postgres_store::diagnostics::DriverError;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

use crate::auth_errors::conversion::AuthFailure;
use crate::auth_invalidation::error::InvalidationError;
use crate::auth_prefetch::permission::lookup::{
    MISSING_PERMISSION_MESSAGE, MISSING_PERMISSION_STATUS, PermissionError,
};
use crate::resolved_key::auth::AuthError;

// The plain Exception get_key_object raises before any read when no database is configured.
pub const NO_DATABASE_MESSAGE: &str =
    "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys";

// A failure that is not a transport fault is parked while the record reaches the bounded lookup.
pub(crate) const PARKED_FAULT: &str =
    "the key read failed for a reason that is not a transport fault";

#[derive(Debug, thiserror::Error)]
pub enum KeyLookupError {
    #[error("No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys")]
    NoDatabase,
    #[error("Key doesn't exist in cache + check_cache_only=True. key={key}.")]
    NotInCache { key: String },
    #[error(
        "Authentication Error, Invalid proxy server token passed. key={key}, not found in db. Create key via `/key/generate` call."
    )]
    MissingKey { key: String },
    #[error(transparent)]
    Read(#[from] KeyReadError),
    #[error(transparent)]
    Lookup(#[from] DbLookupError),
    #[error("the key row did not validate")]
    Row(#[source] Box<AuthError>),
    #[error("the key row is not a JSON object")]
    NotAnObject,
    #[error("the key row did not serialize")]
    Json(#[source] serde_json::Error),
    #[error(transparent)]
    Permission(#[from] PermissionError),
    #[error("the key cache write failed")]
    CacheWrite(#[source] Box<InvalidationError>),
}

impl KeyLookupError {
    // B757's translation: the missing-row ProxyException, then the database outage, then its generic 401.
    #[must_use]
    pub fn failure(&self) -> AuthFailure {
        match self {
            Self::MissingKey { key } => AuthFailure::Proxy {
                exception: missing_key_exception(key),
                invalid_virtual_key: false,
            },
            Self::Read(error) => match error {
                KeyReadError::Database(DatabaseFailure::Reported(fault)) => {
                    AuthFailure::Database(fault.clone())
                }
                KeyReadError::Database(DatabaseFailure::Driver(driver)) => {
                    AuthFailure::Database(db_fault(ErrorType::RawQuery, driver.to_string()))
                }
                KeyReadError::MissingToken => http_failure(error.to_string(), 400),
                KeyReadError::View(_)
                | KeyReadError::Reconnect(_)
                | KeyReadError::Metrics { .. } => other_failure(error.to_string()),
            },
            Self::Lookup(error) => lookup_failure(error),
            Self::Permission(error) => match error {
                PermissionError::Repository(fault) => match error.database_fault() {
                    Some(database) => AuthFailure::Database(database),
                    None => other_failure(fault.to_string()),
                },
                PermissionError::MissingInDatabaseOnly => http_failure(
                    MISSING_PERMISSION_MESSAGE.to_owned(),
                    MISSING_PERMISSION_STATUS,
                ),
                PermissionError::Lookup(lookup) => lookup_failure(lookup),
                PermissionError::NoDatabase
                | PermissionError::Cache(_)
                | PermissionError::Validation(_) => other_failure(error.to_string()),
            },
            Self::NoDatabase
            | Self::NotInCache { .. }
            | Self::Row(_)
            | Self::NotAnObject
            | Self::Json(_)
            | Self::CacheWrite(_) => other_failure(self.to_string()),
        }
    }
}

fn missing_key_exception(key: &str) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: format!(
            "Authentication Error, Invalid proxy server token passed. key={key}, not found in db. Create key via `/key/generate` call."
        ),
        error_type: ProxyErrorTypes::TokenNotFoundInDb.to_string(),
        param: Some("key".to_owned()),
        code: ExceptionCode::Status(401),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

fn lookup_failure(error: &DbLookupError) -> AuthFailure {
    if let DbLookupError::Database(fault) = error {
        return AuthFailure::Database(fault.clone());
    }
    AuthFailure::Database(db_fault(ErrorType::Deadline, error.to_string()))
}

fn http_failure(detail: String, status_code: u16) -> AuthFailure {
    AuthFailure::Http {
        detail,
        param: None,
        status_code,
        invalid_virtual_key: false,
    }
}

fn other_failure(message: String) -> AuthFailure {
    AuthFailure::Other {
        message,
        param: None,
    }
}

fn db_fault(error_type: ErrorType, message: String) -> DbError {
    DbError {
        error_type,
        message,
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

// B860's driver error already hides its raw text, so the transport test reads the driver directly.
pub(crate) fn transport_fault(error: &KeyReadError) -> Option<DbError> {
    match error {
        KeyReadError::Database(DatabaseFailure::Reported(fault)) => {
            is_database_transport_error(fault).then(|| fault.clone())
        }
        KeyReadError::Database(DatabaseFailure::Driver(driver)) => {
            driver_is_transport(driver).then(|| db_fault(ErrorType::Connect, driver.to_string()))
        }
        KeyReadError::MissingToken
        | KeyReadError::View(_)
        | KeyReadError::Reconnect(_)
        | KeyReadError::Metrics { .. } => None,
    }
}

fn driver_is_transport(error: &DriverError) -> bool {
    if let Some(code) = error.driver.code() {
        return classify_sqlstate(code) == NativeFault::ConnectionUnavailable;
    }
    error.driver.is_closed()
        || error
            .driver
            .source()
            .is_some_and(|source| source.downcast_ref::<IoError>().is_some())
}
