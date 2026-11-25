use betterlite_db::database_availability::chain::Chain;
use betterlite_db::database_availability::classification::is_service_unavailable;
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_db::database_faults::{DbError as DatabaseFault, ErrorType};
use betterlite_db::postgres_store::errors::DbError as NativeDbError;
use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException; // ast-grep-ignore: no-long-names
use thiserror::Error;

use crate::auth_end_users::defaults::MemberBudgetError;
use crate::auth_errors::conversion::AuthFailure;
use crate::auth_key_budgets::key_budget::Denial;
use crate::auth_key_budgets::spend::SpendReadError;
use crate::auth_read::table::AuthReadError;
use crate::membership_load::error::{MembershipLoadError, MembershipReadFailure};
use crate::membership_load::rows::MembershipRowError;

#[derive(Debug, Error)]
pub enum TeamBudgetError {
    #[error(transparent)]
    Denied(#[from] Denial),
    #[error(transparent)]
    Spend(#[from] SpendReadError),
    #[error(transparent)]
    Membership(#[from] MembershipLoadError),
    #[error(transparent)]
    DefaultBudget(#[from] MemberBudgetError),
    // the client sees B747's safe message; the internal message names the user, team and allowlist for the failure log
    #[error("the team member's model scope does not allow the requested model")]
    // ast-grep-ignore: no-long-names
    ModelDenied(Box<ModelAccessDeniedProxyException>),
    // _can_object_call_model raises a plain Exception here, which the member scope does not catch
    #[error("Unable to parse model, max fallback depth exceeded - received model: {model}")]
    RecurseDepth { model: String },
}

// auth_exception_handler.py asks is_database_service_unavailable_error before its 401, so a membership outage answers B1217's 503
#[must_use]
pub fn membership_failure(error: &MembershipLoadError) -> AuthFailure {
    membership_outage(error).map_or_else(
        || AuthFailure::Other {
            message: error.to_string(),
            param: None,
        },
        AuthFailure::Database,
    )
}

// the failure rides inside transparent wrappers, so every link is unwrapped here
fn membership_outage(error: &MembershipLoadError) -> Option<DatabaseFault> {
    match error {
        MembershipLoadError::Read(MembershipReadFailure::Access(fault)) => native_outage(fault),
        MembershipLoadError::Read(MembershipReadFailure::Table { source }) => read_outage(source),
        MembershipLoadError::Shared(shared) => membership_outage(shared),
        MembershipLoadError::NoDatabase
        | MembershipLoadError::CacheRead(_)
        | MembershipLoadError::CacheWrite(_)
        | MembershipLoadError::Decode(_)
        | MembershipLoadError::LoadLost => None,
    }
}

fn read_outage(read: &AuthReadError<MembershipRowError>) -> Option<DatabaseFault> {
    match read {
        AuthReadError::Lookup(DbLookupError::Database(fault)) => {
            outage_first(fault).then_some(fault.clone())
        }
        // a lookup deadline is a DBLookupDeadlineExceeded, which subclasses asyncio.TimeoutError
        AuthReadError::Lookup(DbLookupError::Deadline(_) | DbLookupError::Elapsed(_)) => {
            Some(database_fault(
                ErrorType::Deadline,
                "the team membership lookup exceeded its deadline".to_owned(),
            ))
        }
        AuthReadError::Lookup(DbLookupError::Task(_)) => None,
        AuthReadError::Repository(fault) => native_outage(fault),
    }
}

// database_exception classifies only the first link of the chain, so an outage deeper in it does not become the 503
fn outage_first(fault: &DatabaseFault) -> bool {
    Chain::from_error(fault)
        .failures
        .first()
        .is_some_and(is_service_unavailable)
}

fn native_outage<Failure>(fault: &NativeDbError<Failure>) -> Option<DatabaseFault> {
    if !is_native_transport_error(fault) {
        return None;
    }
    Some(database_fault(ErrorType::Connect, fault.to_string()))
}

// the projection reads only the error type and the message, since a membership failure carries no chain
const fn database_fault(error_type: ErrorType, message: String) -> DatabaseFault {
    DatabaseFault {
        error_type,
        message,
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}
