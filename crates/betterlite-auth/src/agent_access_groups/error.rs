use crate::auth_team_lookup::error::TeamLookupError;

#[derive(Debug, thiserror::Error)]
pub enum AccessGroupsError {
    #[error("agent access group {access_group_id} could not be loaded")]
    Read {
        access_group_id: String,
        #[source]
        source: Box<TeamLookupError>,
    },
}
