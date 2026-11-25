use betterlite_auth::ui_session_identity::UI_SESSION_TOKEN_TEAM_ID;
use indexmap::IndexSet;
use tracing::warn;

use super::{GrantReads, ServerGrants};
use crate::auth::object_permission_hydration::reads::ObjectPermissionReads as _;

impl<Reads> ServerGrants<Reads>
where
    Reads: GrantReads,
{
    // _resolve_user_team_ids: a lookup blip narrows to the caller's direct grants instead of raising
    pub async fn user_team_ids(&self, user_id: &str, requires_fresh_policy: bool) -> Vec<String> {
        if !self.reads.permissions().has_database() {
            return Vec::new();
        }
        let teams = match self
            .reads
            .user_team_ids(user_id, requires_fresh_policy)
            .await
        {
            Ok(found) => found.unwrap_or_default(),
            Err(error) => {
                warn!("Failed to resolve user teams for MCP grant: {error}");
                return Vec::new();
            }
        };
        let distinct: IndexSet<String> = teams
            .into_iter()
            .filter(|team_id| !team_id.is_empty() && team_id.as_str() != UI_SESSION_TOKEN_TEAM_ID)
            .collect();
        distinct.into_iter().collect()
    }
}
