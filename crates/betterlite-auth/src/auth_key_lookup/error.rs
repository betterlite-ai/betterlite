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

use crate::auth_errors::conversion::{AuthFailure, as_proxy_exception};
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
            Self::Read(error) => read_failure(error),
            Self::Lookup(error) => lookup_failure(error),
            Self::Permission(error) => permission_failure(error),
            Self::NoDatabase
            | Self::NotInCache { .. }
            | Self::Row(_)
            | Self::NotAnObject
            | Self::Json(_)
            | Self::CacheWrite(_) => AuthFailure::Other {
                message: self.to_string(),
                param: None,
            },
        }
    }
}

#[must_use]
pub fn proxy_exception(error: &KeyLookupError) -> ProxyException {
    as_proxy_exception(&error.failure()).exception().clone()
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

fn read_failure(error: &KeyReadError) -> AuthFailure {
    match error {
        KeyReadError::Database(DatabaseFailure::Reported(fault)) => {
            AuthFailure::Database(fault.clone())
        }
        KeyReadError::Database(DatabaseFailure::Driver(driver)) => {
            AuthFailure::Database(raw_query_fault(driver))
        }
        KeyReadError::MissingToken => AuthFailure::Http {
            detail: error.to_string(),
            param: None,
            status_code: 400,
            invalid_virtual_key: false,
        },
        KeyReadError::View(_) | KeyReadError::Reconnect(_) | KeyReadError::Metrics { .. } => {
            AuthFailure::Other {
                message: error.to_string(),
                param: None,
            }
        }
    }
}

fn lookup_failure(error: &DbLookupError) -> AuthFailure {
    match error {
        DbLookupError::Database(fault) => AuthFailure::Database(fault.clone()),
        DbLookupError::Deadline(_) | DbLookupError::Elapsed(_) | DbLookupError::Task(_) => {
            AuthFailure::Database(deadline_fault(error))
        }
    }
}

fn permission_failure(error: &PermissionError) -> AuthFailure {
    match error {
        PermissionError::MissingInDatabaseOnly => AuthFailure::Http {
            detail: MISSING_PERMISSION_MESSAGE.to_owned(),
            param: None,
            status_code: MISSING_PERMISSION_STATUS,
            invalid_virtual_key: false,
        },
        PermissionError::Lookup(DbLookupError::Database(fault)) => {
            AuthFailure::Database(fault.clone())
        }
        PermissionError::Lookup(
            deadline @ (DbLookupError::Deadline(_)
            | DbLookupError::Elapsed(_)
            | DbLookupError::Task(_)),
        ) => AuthFailure::Database(deadline_fault(deadline)),
        PermissionError::Repository(fault) => match error.database_fault() {
            Some(fault) => AuthFailure::Database(fault),
            None => AuthFailure::Other {
                message: fault.to_string(),
                param: None,
            },
        },
        PermissionError::NoDatabase
        | PermissionError::Cache(_)
        | PermissionError::Validation(_) => AuthFailure::Other {
            message: error.to_string(),
            param: None,
        },
    }
}

fn deadline_fault(error: &DbLookupError) -> DbError {
    DbError {
        error_type: ErrorType::Deadline,
        message: error.to_string(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

fn raw_query_fault(driver: &DriverError) -> DbError {
    DbError {
        error_type: ErrorType::RawQuery,
        message: driver.to_string(),
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
            driver_is_transport(driver).then(|| connect_fault(driver))
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

fn connect_fault(driver: &DriverError) -> DbError {
    DbError {
        error_type: ErrorType::Connect,
        message: driver.to_string(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}
