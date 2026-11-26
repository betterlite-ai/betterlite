use std::fmt;
use std::future::Future;

use betterlite_error::proxy_exception::ProxyException;

use crate::auth_end_users::check::EndUserBudgetExceeded;
use crate::auth_org_lookup::error::OrgLookupError;
use crate::auth_team_lookup::error::TeamLookupError;

// the exceptions _safe_fetch re-raises: LiteLLM's HTTPException, ProxyException and BudgetExceededError
pub trait FetchFailure: fmt::Display {
    fn is_auth_failure(&self) -> bool;
}

impl FetchFailure for ProxyException {
    fn is_auth_failure(&self) -> bool {
        true
    }
}

impl FetchFailure for EndUserBudgetExceeded {
    fn is_auth_failure(&self) -> bool {
        true
    }
}

impl FetchFailure for TeamLookupError {
    fn is_auth_failure(&self) -> bool {
        self.http().is_some()
    }
}

impl FetchFailure for OrgLookupError {
    fn is_auth_failure(&self) -> bool {
        self.http().is_some()
    }
}

// _safe_fetch: an authorization failure propagates, every other failure is logged and becomes None
pub async fn safe_fetch<T, E, F>(label: &str, future: F) -> Result<Option<T>, E>
where
    E: FetchFailure,
    F: Future<Output = Result<T, E>>,
{
    match future.await {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.is_auth_failure() => {
            tracing::debug!(label, error = %error, "centralized auth: fetch failed");
            Err(error)
        }
        Err(error) => {
            tracing::debug!(label, error = %error, "centralized auth: fetch swallowed");
            Ok(None)
        }
    }
}
