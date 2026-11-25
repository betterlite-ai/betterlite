use std::error::Error as StdError;

use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_db::mcp::table_access::types::McpTableError;
use thiserror::Error;

use crate::access_group_servers::AccessGroupServersError;
use crate::auth::object_permission_hydration::error::{
    EntitlementError, UnloadableEntitlementError,
};

#[derive(Debug, Error)]
pub enum GrantReadError {
    #[error("the team could not be read: {0}")]
    Team(#[source] TeamLookupError),
    #[error("the user could not be read: {0}")]
    User(#[source] UserLookupError),
    #[error("the access group could not be read: {0}")]
    AccessGroup(#[source] TeamLookupError),
    #[error("the MCP servers for the access groups could not be read: {0}")]
    AccessGroupServers(#[source] Box<AccessGroupServersError<McpTableError>>),
}

#[derive(Debug, Error)]
pub enum ServerGrantsError<Source>
where
    Source: StdError + 'static,
{
    #[error(transparent)]
    Unloadable(UnloadableEntitlementError<Source>),
    #[error("the MCP object permission read failed: {source}")]
    Permission {
        #[source]
        source: Source,
    },
    #[error("an MCP grant read failed: {source}")]
    Read {
        #[from]
        source: GrantReadError,
    },
    // get_team_object raises for a team the database does not have
    #[error("team {team_id:?} does not exist")]
    TeamMissing { team_id: String },
}

impl<Source> From<EntitlementError<Source>> for ServerGrantsError<Source>
where
    Source: StdError + 'static,
{
    fn from(error: EntitlementError<Source>) -> Self {
        match error {
            EntitlementError::Read { source } => Self::Permission { source },
            EntitlementError::Unloadable(unloadable) => Self::Unloadable(unloadable),
        }
    }
}
