use std::error::Error;

use betterlite_error::proxy_exception::ProxyException;

use crate::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use crate::auth_org_lookup::error::{OrgLookupError, as_auth_failure as org_auth_failure};
use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_user_lookup::error::UserLookupError;
use crate::end_user_registry::errors::RegistryError;
use crate::membership_load::error::MembershipLoadError;
use crate::model_grant_intersection::sources::ServingFailure;

// Every way the walks leave; only a lookup's HTTPException carries a status, as auth_exception_handler reads it.
#[derive(Debug, thiserror::Error)]
pub enum IntersectionError {
    #[error(transparent)]
    Team(#[from] TeamLookupError),
    #[error(transparent)]
    User(#[from] UserLookupError),
    #[error(transparent)]
    Membership(#[from] MembershipLoadError),
    #[error(transparent)]
    Project(#[from] OrgLookupError),
    #[error("the model access group registry could not be read")]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    Groups(#[from] ServingFailure),
}

impl IntersectionError {
    // the HTTPException's status_code; None is a plain Exception, which B757 answers with its 401
    #[must_use]
    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::Team(error) => error.http().map(|(status, _detail)| status),
            Self::User(_) | Self::Membership(_) | Self::Registry(_) | Self::Groups(_) => None,
            Self::Project(error) => error.http(),
        }
    }

    #[must_use]
    pub fn as_auth_failure(&self) -> AuthFailure {
        match self {
            Self::Team(error) => team_failure(error),
            Self::Project(error) => org_auth_failure(error),
            Self::User(_) | Self::Membership(_) | Self::Registry(_) | Self::Groups(_) => {
                AuthFailure::Other {
                    message: self.to_string(),
                    param: None,
                }
            }
        }
    }

    #[must_use]
    pub fn proxy_exception(&self) -> ProxyException {
        let failure = self.as_auth_failure();
        as_proxy_exception(&failure).exception().clone()
    }
}

fn team_failure(error: &TeamLookupError) -> AuthFailure {
    match error.http() {
        Some((status_code, _detail)) => AuthFailure::Http {
            detail: error.to_string(),
            param: None,
            status_code,
            invalid_virtual_key: false,
        },
        None => plain_failure(error),
    }
}

fn plain_failure(error: &(dyn Error + 'static)) -> AuthFailure {
    AuthFailure::Other {
        message: error.to_string(),
        param: None,
    }
}
