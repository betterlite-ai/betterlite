use betterlite_auth::ui_session_identity::UI_SESSION_TOKEN_TEAM_ID;

use super::auth::GrantAuth;
use super::error::ServerGrantsError;
use super::{GrantReads, GrantsError, ServerGrants};

// _team_ids_for_mcp_grant: only the caller's own team_id counts, never the other teams of its user
#[must_use]
pub fn team_id_for_grant(auth: Option<&GrantAuth>) -> Option<&str> {
    let team_id = auth?.mcp.team_id.as_deref()?;
    if team_id.is_empty() || team_id == UI_SESSION_TOKEN_TEAM_ID {
        return None;
    }
    Some(team_id)
}

// the row and its guards behind _allowed_mcp_servers_for_single_team; a missing row is an error under the fresh policy
pub(super) async fn team_grant<Reads>(
    grants: &ServerGrants<Reads>,
    team_id: &str,
    requires_fresh_policy: bool,
) -> Result<Vec<String>, GrantsError<Reads>>
where
    Reads: GrantReads,
{
    let found = grants.reads.team(team_id, requires_fresh_policy).await?;
    let Some(team) = found else {
        return Err(ServerGrantsError::TeamMissing {
            team_id: team_id.to_owned(),
        });
    };
    // a keyless admitted subject has no key whose policy gate would catch a blocked team
    if team.blocked {
        return Ok(Vec::new());
    }
    let group_servers = grants
        .reads
        .access_group_server_ids(&team.access_group_ids, requires_fresh_policy)
        .await?;
    grants
        .team_granted_servers(
            team.object_permission.as_ref(),
            group_servers,
            requires_fresh_policy,
        )
        .await
}
