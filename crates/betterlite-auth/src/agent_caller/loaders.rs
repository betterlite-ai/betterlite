use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use tracing::debug;

use super::error::AgentCallerError;
use super::reads::CallerReads;
use crate::resolved_key::auth::UserAPIKeyAuth;

mod tests;

pub struct AgentCallerLoaders<Reads> {
    pub reads: Reads,
}

impl<Reads: CallerReads> AgentCallerLoaders<Reads> {
    // a caller team that cannot be read is an error, never "no team": that would drop the cap
    pub async fn load_agent_caller_team(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, AgentCallerError> {
        let caller = auth.extra.agent_caller.as_ref();
        let Some(team_id) = caller.and_then(|found| found.team_id.as_deref()) else {
            return Ok(None);
        };
        self.reads
            .team(team_id)
            .await
            .map(Some)
            .map_err(|source| AgentCallerError::Team {
                team_id: team_id.to_owned(),
                source: Box::new(source),
            })
    }

    // a user id with no row narrows nothing, while a failed read is passed up
    pub async fn load_agent_caller_user(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> Result<Option<LiteLLMUserTable>, AgentCallerError> {
        let caller = auth.extra.agent_caller.as_ref();
        let Some(user_id) = caller.and_then(|found| found.user_id.as_deref()) else {
            return Ok(None);
        };
        match self.reads.user(user_id).await {
            Ok(Some(user)) => Ok(Some(user)),
            Ok(None) => {
                debug!(
                    user_id,
                    "agent caller user {user_id} not found; no user ceiling applied"
                );
                Ok(None)
            }
            Err(source) => Err(AgentCallerError::User {
                user_id: user_id.to_owned(),
                source: Box::new(source),
            }),
        }
    }
}
