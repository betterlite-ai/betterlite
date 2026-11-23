use std::collections::BTreeMap;
use std::convert::Infallible;

use betterlite_db::database_availability::chain::{Chain, has_outage_in_chain};
use betterlite_db::database_availability::message::TRANSIENT_MESSAGE;
use betterlite_db::database_availability::policy::PolicyError;
use betterlite_db::database_availability::projection::SERVICE_UNAVAILABLE_STATUS;
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_db::postgres_store::errors::DbError as NativeDbError;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use deadpool_postgres::PoolError;
use tokio_postgres::error::DbError as DriverDbError;

use super::ports::StoreFailure;
use crate::auth_cache::AuthCacheError;
use crate::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use crate::auth_read::table::AuthReadError;

// exception_handler.py's one message marker that is not a transport word
const STALE_PLAN_MESSAGE: &str = "cached plan must not change result type";

// What leaves the module. http() carries LiteLLM's HTTPException status, and B757 renders every variant.
#[derive(Debug, thiserror::Error)]
pub enum OrgLookupError {
    #[error("No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys")]
    NoDatabase,
    // the cache read sits before the source's try block, so its failure is never relabeled
    #[error("the organization cache read failed")]
    CacheRead(#[source] Box<AuthCacheError>),
    // OrganizationNotFoundError: the database answered and the row is absent
    #[error(
        "Organization doesn't exist in db. Organization={org_id}. Create organization via `/organization/new` call."
    )]
    NotFound { org_id: String },
    #[error(
        "Organization with alias '{org_alias}' doesn't exist in db. Create organization via `/organization/new` call."
    )]
    AliasNotFound { org_alias: String },
    #[error(
        "Multiple organizations found with alias '{org_alias}'. Please use org_id_jwt_field instead or ensure organization aliases are unique."
    )]
    AliasAmbiguous { org_alias: String },
    #[error("Error looking up organization by alias '{org_alias}': {cause}")]
    AliasFailure {
        org_alias: String,
        #[source]
        cause: Box<OrgFailure>,
    },
    #[error(transparent)]
    Failure(#[from] OrgFailure),
}

// The failures inside the source's try blocks, kept as the cause of the error they become.
#[derive(Debug, thiserror::Error)]
pub enum OrgFailure {
    #[error("the organization table read failed")]
    Store(#[source] StoreFailure),
    #[error("the stored row did not validate")]
    Row(#[source] Box<serde_json::Error>),
    #[error("the organization cache failed")]
    Cache(#[source] Box<AuthCacheError>),
    #[error("allow_requests_on_db_unavailable is neither a boolean nor a string")]
    Policy(#[source] PolicyError),
}

impl OrgLookupError {
    // the HTTPException's status_code; None is a plain Exception, which B757 answers with its 401
    #[must_use]
    pub const fn http(&self) -> Option<u16> {
        match self {
            Self::AliasNotFound { .. } => Some(404),
            Self::AliasAmbiguous { .. } => Some(400),
            Self::AliasFailure { .. } => Some(500),
            Self::NoDatabase | Self::CacheRead(_) | Self::NotFound { .. } | Self::Failure(_) => {
                None
            }
        }
    }
}

// B757's translation: the HTTPException first, then B1217's 503, then its generic 401.
#[must_use]
pub fn as_auth_failure(error: &OrgLookupError) -> AuthFailure {
    if let Some(status_code) = error.http() {
        return AuthFailure::Http {
            detail: error.to_string(),
            param: None,
            status_code,
            invalid_virtual_key: false,
        };
    }
    if is_outage(error) {
        return AuthFailure::Proxy {
            exception: outage_exception(),
            invalid_virtual_key: false,
        };
    }
    AuthFailure::Other {
        message: error.to_string(),
        param: None,
    }
}

#[must_use]
pub fn proxy_exception(error: &OrgLookupError) -> ProxyException {
    as_proxy_exception(&as_auth_failure(error))
        .exception()
        .clone()
}

// PrismaDBExceptionHandler.is_database_service_unavailable_error_in_chain: the alias catch relabels an
// outage to its 500, so the status check above answers first and this only reads the wrapper's failures
#[must_use]
pub fn is_outage(error: &OrgLookupError) -> bool {
    match error {
        Self::Failure(failure) => failure_is_outage(failure),
        Self::AliasFailure { cause, .. } => failure_is_outage(cause),
        Self::NoDatabase
        | Self::CacheRead(_)
        | Self::NotFound { .. }
        | Self::AliasNotFound { .. }
        | Self::AliasAmbiguous { .. } => false,
    }
}

fn failure_is_outage(failure: &OrgFailure) -> bool {
    match failure {
        OrgFailure::Store(read) => read_is_outage(read),
        // a decode, cache or policy failure is never a database outage
        OrgFailure::Row(_) | OrgFailure::Cache(_) | OrgFailure::Policy(_) => false,
    }
}

fn read_is_outage(read: &AuthReadError<Infallible>) -> bool {
    match read {
        AuthReadError::Lookup(DbLookupError::Database(fault)) => {
            has_outage_in_chain(&Chain::from_error(fault))
        }
        // a lookup deadline is a DBLookupDeadlineExceeded, which subclasses asyncio.TimeoutError
        AuthReadError::Lookup(DbLookupError::Deadline(_) | DbLookupError::Elapsed(_)) => true,
        AuthReadError::Lookup(DbLookupError::Task(_)) => false,
        AuthReadError::Repository(fault) => native_is_outage(fault),
    }
}

fn native_is_outage(fault: &NativeDbError<Infallible>) -> bool {
    if is_native_transport_error(fault) {
        return true;
    }
    driver_database_error(fault).is_some_and(|error| {
        error
            .message()
            .to_ascii_lowercase()
            .contains(STALE_PLAN_MESSAGE)
    })
}

// the driver keeps the server's message off its own Display, so the database error is read directly
fn driver_database_error(fault: &NativeDbError<Infallible>) -> Option<&DriverDbError> {
    match fault {
        NativeDbError::Backend { source }
        | NativeDbError::SqlState { source, .. }
        | NativeDbError::UniqueViolation { source, .. } => source.driver.as_db_error(),
        NativeDbError::Pool { source } => match &source.driver {
            PoolError::Backend(driver) => driver.as_db_error(),
            PoolError::Timeout(_)
            | PoolError::Closed
            | PoolError::NoRuntimeSpecified
            | PoolError::PostCreateHook(_) => None,
        },
        NativeDbError::NoDbConnected
        | NativeDbError::InvalidRecord { .. }
        | NativeDbError::CreateNull => None,
    }
}

// B1217's 503 envelope; service_unavailable_error needs a chain of prisma faults, which a native
// pool or driver fault is not, so the projection's constants are used directly
fn outage_exception() -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: TRANSIENT_MESSAGE.to_owned(),
        error_type: ProxyErrorTypes::NoDbConnection.to_string(),
        param: Some("None".to_owned()),
        code: ExceptionCode::Status(i64::from(SERVICE_UNAVAILABLE_STATUS)),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}
