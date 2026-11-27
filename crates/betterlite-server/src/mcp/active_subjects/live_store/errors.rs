use std::convert::Infallible;
use std::error::Error;
use std::io::Error as IoError;

use betterlite_auth::auth_errors::conversion::AuthFailure;
use betterlite_auth::auth_key_lookup::error::KeyLookupError;
use betterlite_auth::auth_read::table::AuthReadError;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::auth_user_lookup::ports::PortError;
use betterlite_auth::auth_user_lookup::rows::UserRowError;
use betterlite_auth::identity_store::error::IdentityStoreError;
use betterlite_db::database_availability::chain::Chain;
use betterlite_db::database_availability::classification::{
    STALE_PLAN_MARKER, is_permanent_fault, is_service_unavailable,
};
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::DbError;
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_db::postgres_store::errors::DbError as NativeDbError;
use betterlite_db::user_repository::RowDecodeError;

use super::super::{IdentityLookup, SubjectReadError};

const MAX_CHAIN_LINKS: usize = 32;

// _reload_active_key_by_hash classifies the exception itself, not its __cause__ chain
pub(super) fn key_read_error(error: &KeyLookupError) -> SubjectReadError {
    // a lookup timeout still carries the fault that caused it, which the chain search still reads
    if let KeyLookupError::Lookup(lookup @ DbLookupError::Deadline(_)) = error {
        return deadline_error(lookup);
    }
    match error.failure() {
        AuthFailure::Proxy { .. } | AuthFailure::Http { .. } => SubjectReadError::Refused,
        AuthFailure::Database(fault) => {
            let chain = Chain::from_error(&fault);
            if chain.failures.first().is_some_and(is_service_unavailable) {
                outage_error(&chain)
            } else {
                SubjectReadError::Other
            }
        }
        AuthFailure::BudgetExceeded { .. }
        | AuthFailure::ModelAccessDenied { .. }
        | AuthFailure::Other { .. } => SubjectReadError::Other,
    }
}

fn outage_error(chain: &Chain) -> SubjectReadError {
    if chain.find_outage().is_some_and(is_permanent_fault) {
        return SubjectReadError::Faulted;
    }
    SubjectReadError::Unavailable
}

fn deadline_error(lookup: &DbLookupError) -> SubjectReadError {
    if matches!(lookup_outage(lookup), LinkOutage::Permanent) {
        return SubjectReadError::Faulted;
    }
    SubjectReadError::Unavailable
}

// load_active_user_by_id walks the whole chain, because get_user_object wraps a failure in a ValueError
pub(super) fn user_read_error(error: &UserLookupError) -> SubjectReadError {
    match error {
        UserLookupError::NotFound(_) | UserLookupError::NoDatabase => SubjectReadError::Other,
        UserLookupError::Unavailable(cause) => {
            user_outage(cause.as_ref()).unwrap_or(SubjectReadError::Unavailable)
        }
        UserLookupError::UserMissing(failure) => {
            user_outage(failure.cause.as_ref()).unwrap_or(SubjectReadError::Other)
        }
        UserLookupError::PreLookup(cause) => {
            user_outage(cause.as_ref()).unwrap_or(SubjectReadError::Other)
        }
    }
}

// find_database_service_unavailable_error_in_chain: a permanent fault outranks a transient one
fn user_outage(error: &(dyn Error + 'static)) -> Option<SubjectReadError> {
    let mut root = Some(error);
    let mut transient = false;
    for _ in 0..MAX_CHAIN_LINKS {
        let Some(link) = root else {
            break;
        };
        match link_outage(link) {
            LinkOutage::None => {}
            LinkOutage::Transient => transient = true,
            LinkOutage::Permanent => return Some(SubjectReadError::Faulted),
        }
        root = link.source();
    }
    transient.then_some(SubjectReadError::Unavailable)
}

enum LinkOutage {
    None,
    Transient,
    Permanent,
}

// a transparent wrapper hides its payload from source(), so each wrapper's own type is inspected here
fn link_outage(link: &(dyn Error + 'static)) -> LinkOutage {
    if let Some(fault) = link.downcast_ref::<DbError>() {
        return fault_outage(fault);
    }
    if let Some(lookup) = link.downcast_ref::<DbLookupError>() {
        return lookup_outage(lookup);
    }
    if let Some(read) = link.downcast_ref::<AuthReadError<UserRowError>>() {
        return auth_read_outage(read);
    }
    if let Some(port) = link.downcast_ref::<PortError>()
        && let Some(fault) = port.database_fault.as_deref()
    {
        return fault_outage(fault);
    }
    if let Some(native) = link.downcast_ref::<NativeDbError<UserRowError>>() {
        return native_outage(native);
    }
    if let Some(native) = link.downcast_ref::<NativeDbError<RowDecodeError>>() {
        return native_outage(native);
    }
    if let Some(native) = link.downcast_ref::<NativeDbError<Infallible>>() {
        return native_outage(native);
    }
    if link.downcast_ref::<IoError>().is_some() {
        return LinkOutage::Transient;
    }
    if link
        .to_string()
        .to_ascii_lowercase()
        .contains(STALE_PLAN_MARKER)
    {
        return LinkOutage::Transient;
    }
    LinkOutage::None
}

fn fault_outage(fault: &DbError) -> LinkOutage {
    let chain = Chain::from_error(fault);
    let Some(outage) = chain.find_outage() else {
        return LinkOutage::None;
    };
    if is_permanent_fault(outage) {
        LinkOutage::Permanent
    } else {
        LinkOutage::Transient
    }
}

fn lookup_outage(lookup: &DbLookupError) -> LinkOutage {
    match lookup {
        DbLookupError::Database(fault) => fault_outage(fault),
        DbLookupError::Deadline(exceeded) => {
            let deeper = exceeded
                .cause
                .as_deref()
                .map_or(LinkOutage::None, lookup_outage);
            if matches!(deeper, LinkOutage::Permanent) {
                LinkOutage::Permanent
            } else {
                LinkOutage::Transient
            }
        }
        DbLookupError::Elapsed(_) => LinkOutage::Transient,
        DbLookupError::Task(_) => LinkOutage::None,
    }
}

fn auth_read_outage(read: &AuthReadError<UserRowError>) -> LinkOutage {
    match read {
        AuthReadError::Lookup(lookup) => lookup_outage(lookup),
        AuthReadError::Repository(fault) => native_outage(fault),
    }
}

fn native_outage<Failure>(fault: &NativeDbError<Failure>) -> LinkOutage {
    if is_native_transport_error(fault) {
        return LinkOutage::Transient;
    }
    LinkOutage::None
}

// _opaque_bearer_is_gateway_credential: anything but a missing key must not permit the cookie fallback
pub(super) fn identity_error(error: &IdentityStoreError) -> IdentityLookup {
    match error {
        IdentityStoreError::KeyLookup(KeyLookupError::MissingKey { .. }) => {
            IdentityLookup::NotFound
        }
        IdentityStoreError::KeyLookup(
            KeyLookupError::NoDatabase
            | KeyLookupError::NotInCache { .. }
            | KeyLookupError::Read(_)
            | KeyLookupError::Lookup(_)
            | KeyLookupError::Row(_)
            | KeyLookupError::NotAnObject
            | KeyLookupError::Json(_)
            | KeyLookupError::Permission(_)
            | KeyLookupError::CacheWrite(_),
        )
        | IdentityStoreError::Cache(_)
        | IdentityStoreError::PrincipalMissingSourceKey
        | IdentityStoreError::MissingPorts { .. } => {
            tracing::debug!(error = %error, "OAuth bearer ownership could not be checked");
            IdentityLookup::Faulted
        }
    }
}
