use std::convert::Infallible;
use std::error::Error;
use std::io::Error as IoError;

use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_db::database_faults::{
    DbError as DatabaseFault, is_database_transport_error, is_engine_internal_error,
    is_infrastructure_error,
};
use betterlite_db::postgres_store::errors::DbError as NativeDbError;
use betterlite_db::user_repository::RowDecodeError;
use deadpool_postgres::PoolError;

use super::ports::PortError;
use super::rows::UserRowError;
use crate::auth_identity::errors::AuthError;
use crate::auth_read::table::AuthReadError;

// exception_handler.py's one message marker that is not a transport word
pub const STALE_PLAN_MESSAGE: &str = "cached plan must not change result type";
const MAX_CHAIN_LINKS: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum UserLookupError {
    #[error(transparent)]
    NotFound(#[from] AuthError),
    #[error("No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys")]
    NoDatabase,
    // _user_read_failure returns the original error when the database itself could not answer
    #[error(transparent)]
    Unavailable(Box<dyn Error + Send + Sync>),
    #[error(transparent)]
    UserMissing(#[from] UserReadFailure),
    // raised before the lookup's catch block: the cache read and the cache-hit backfill
    #[error(transparent)]
    PreLookup(Box<dyn Error + Send + Sync>),
}

// _user_read_failure's ValueError: the guidance a caller sees for any ordinary read failure
#[derive(Debug, thiserror::Error)]
#[error(
    "User doesn't exist in db. 'user_id'={user_id}. Create user via `/user/new` call. Got error - {cause}"
)]
pub struct UserReadFailure {
    pub user_id: String,
    #[source]
    pub cause: Box<dyn Error + Send + Sync>,
}

// PrismaDBExceptionHandler.is_database_service_unavailable_error over the error's cause chain
#[must_use]
pub fn user_read_failure(user_id: &str, cause: Box<dyn Error + Send + Sync>) -> UserLookupError {
    if is_service_unavailable(cause.as_ref()) {
        return UserLookupError::Unavailable(cause);
    }
    UserLookupError::UserMissing(UserReadFailure {
        user_id: user_id.to_owned(),
        cause,
    })
}

fn is_service_unavailable(error: &(dyn Error + 'static)) -> bool {
    let mut link = Some(error);
    for _ in 0..MAX_CHAIN_LINKS {
        let Some(current) = link else {
            return false;
        };
        if unavailable_link(current) {
            return true;
        }
        link = current.source();
    }
    false
}

// a transparent wrapper hides its payload from source(), so each wrapper's own type is inspected here
fn unavailable_link(link: &(dyn Error + 'static)) -> bool {
    if let Some(fault) = link.downcast_ref::<DatabaseFault>() {
        return database_unavailable(fault);
    }
    if let Some(lookup) = link.downcast_ref::<DbLookupError>() {
        return lookup_unavailable(lookup);
    }
    if let Some(read) = link.downcast_ref::<AuthReadError<UserRowError>>() {
        return auth_read_unavailable(read);
    }
    if let Some(port) = link.downcast_ref::<PortError>()
        && port
            .database_fault
            .as_deref()
            .is_some_and(database_unavailable)
    {
        return true;
    }
    if let Some(native) = link.downcast_ref::<NativeDbError<UserRowError>>() {
        return native_unavailable(native);
    }
    if let Some(native) = link.downcast_ref::<NativeDbError<RowDecodeError>>() {
        return native_unavailable(native);
    }
    if let Some(native) = link.downcast_ref::<NativeDbError<Infallible>>() {
        return native_unavailable(native);
    }
    if link.downcast_ref::<IoError>().is_some() {
        return true;
    }
    link.to_string()
        .to_ascii_lowercase()
        .contains(STALE_PLAN_MESSAGE)
}

fn database_unavailable(fault: &DatabaseFault) -> bool {
    is_infrastructure_error(fault.error_type)
        || is_database_transport_error(fault)
        || is_engine_internal_error(fault)
        || fault
            .message
            .to_ascii_lowercase()
            .contains(STALE_PLAN_MESSAGE)
}

fn lookup_unavailable(lookup: &DbLookupError) -> bool {
    match lookup {
        DbLookupError::Database(fault) => database_unavailable(fault),
        DbLookupError::Deadline(_) | DbLookupError::Elapsed(_) => true,
        DbLookupError::Task(_) => false,
    }
}

fn auth_read_unavailable(read: &AuthReadError<UserRowError>) -> bool {
    match read {
        AuthReadError::Lookup(lookup) => lookup_unavailable(lookup),
        AuthReadError::Repository(fault) => native_unavailable(fault),
    }
}

fn native_unavailable<Failure>(error: &NativeDbError<Failure>) -> bool {
    if is_native_transport_error(error) {
        return true;
    }
    native_driver(error).is_some_and(driver_names_stale_plan)
}

// the driver keeps the server's message in its source chain, not in its own Display
fn driver_names_stale_plan(driver: &tokio_postgres::Error) -> bool {
    let mut link = Some(driver as &(dyn Error + 'static));
    while let Some(current) = link {
        if current
            .to_string()
            .to_ascii_lowercase()
            .contains(STALE_PLAN_MESSAGE)
        {
            return true;
        }
        link = current.source();
    }
    false
}

// DriverError hides the driver text from the source chain, so a stale-plan fault is read here
const fn native_driver<Failure>(error: &NativeDbError<Failure>) -> Option<&tokio_postgres::Error> {
    match error {
        NativeDbError::Backend { source }
        | NativeDbError::SqlState { source, .. }
        | NativeDbError::UniqueViolation { source, .. } => Some(&source.driver),
        NativeDbError::Pool { source } => match &source.driver {
            PoolError::Backend(driver) => Some(driver),
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
