use async_trait::async_trait;
use betterlite_db::team_repository::repository::{TeamQueryError, TeamRepository};

use super::error::KeyAccessError;

// the team read validate_team_id_used_in_service_account_request makes, injected so a route can pass its repository
#[async_trait]
pub trait TeamDirectory: Send + Sync {
    async fn team_exists(&self, team_id: &str) -> Result<bool, TeamQueryError>;
}

// LiteLLM reads the raw table row through _prisma_table, never converting it into the team model
#[async_trait]
impl TeamDirectory for TeamRepository<'_> {
    async fn team_exists(&self, team_id: &str) -> Result<bool, TeamQueryError> {
        Ok(self.table().find_unique(team_id).await?.is_some())
    }
}

// a missing team_id, then a missing database, then an unknown team, each before any generation work
pub async fn validate_service_account_team(
    team_id: Option<&str>,
    teams: Option<&dyn TeamDirectory>,
) -> Result<(), KeyAccessError> {
    let Some(team) = team_id else {
        return Err(KeyAccessError::ServiceTeamRequired);
    };
    let Some(directory) = teams else {
        return Err(KeyAccessError::ServiceDatabaseRequired);
    };
    let exists = directory
        .team_exists(team)
        .await
        .map_err(|error| KeyAccessError::TeamRead(Box::new(error)))?;
    if exists {
        return Ok(());
    }
    Err(KeyAccessError::ServiceTeamUnknown {
        team_id: team.to_owned(),
    })
}
