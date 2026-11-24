use std::num::TryFromIntError;

use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::team_repository::decoder::TeamRecordError;
use betterlite_db::team_rows::decoder::TeamRowError;
use serde::Serialize;

use crate::team_access::error::TeamAccessError;
use crate::team_request_contracts::TeamContractError;

pub mod access;
pub mod conditions;
pub mod lookup;
pub mod read;
pub mod response;
pub mod sql;

mod tests;

// every list_team_v2 and team/list refusal raises HTTPException(detail={"error": ...})
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TeamListRejection {
    #[error("You can only view teams within your organizations.")]
    OrgScope,
    #[error("Only admin users can query all teams/other teams. Your user role={role}")]
    OtherTeams { role: String },
    #[error("User not found, passed user_id={user_id}")]
    UserNotFound { user_id: String },
    #[error("User is not a member of team_id={team_id}")]
    NotTeamMember { team_id: String },
}

impl TeamListRejection {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::OrgScope => 403,
            Self::OtherTeams { .. } => 401,
            Self::UserNotFound { .. } | Self::NotTeamMember { .. } => 404,
        }
    }

    #[must_use]
    pub fn detail(&self) -> TeamListRejectionDetail {
        TeamListRejectionDetail {
            error: self.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TeamListRejectionDetail {
    pub error: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TeamListQueryError {
    #[error(transparent)]
    Rejected(#[from] TeamListRejection),
    #[error("the team list query failed")]
    Database(#[from] DbError),
    #[error("a stored team row cannot be decoded")]
    Record(#[from] TeamRecordError),
    #[error("a stored team row cannot be decoded")]
    Row(#[from] TeamRowError),
    #[error("the team list response row cannot be decoded")]
    Contract(#[from] TeamContractError),
    #[error("the user lookup failed")]
    Lookup(#[from] UserLookupError),
    #[error("the caller cannot be read")]
    Caller(#[from] TeamAccessError),
    #[error("a stored access group row cannot be decoded")]
    AccessGroup(#[source] serde_json::Error),
    #[error("a team list count does not fit its integer type")]
    Count {
        #[source]
        source: TryFromIntError,
    },
}
