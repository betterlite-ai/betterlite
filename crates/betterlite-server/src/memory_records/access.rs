use async_trait::async_trait;
use axum::http::StatusCode;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::team_repository::repository::{TeamQueryError, TeamRepository};
use betterlite_db::team_rows::team::TeamBase;

use crate::team_access::access::{TEAM_OR_ORG_ADMIN, TeamAccess};
use crate::team_access::caller::TeamCaller;

use super::caller::{MemoryCaller, is_admin};
use super::error::MemoryHttpError;
use super::models::MemoryRow;

// memory_endpoints.py TeamRepository(prisma_client).find_by_id(team_id, id_field="team_id")
#[async_trait]
pub trait TeamRows: Send + Sync {
    async fn find_team(&self, team_id: &str) -> Result<Option<TeamBase<String>>, TeamQueryError>;
}

#[async_trait]
impl TeamRows for TeamRepository<'_> {
    // the lookup needs a live connection; every test of this module fakes TeamRows
    #[cfg_attr(test, mutants::skip)]
    async fn find_team(&self, team_id: &str) -> Result<Option<TeamBase<String>>, TeamQueryError> {
        let found = self.find_by_id(team_id).await?;
        Ok(found.map(|row| row.fields.base))
    }
}

// memory_endpoints.py _is_team_admin_for: a lookup or access-check failure logs and denies, never grants
async fn is_team_admin_for(
    teams: &dyn TeamRows,
    access: &TeamAccess,
    caller: &MemoryCaller,
    team_id: &str,
) -> bool {
    let team = match teams.find_team(team_id).await {
        Ok(team) => team,
        Err(error) => {
            tracing::error!("Error loading team for write-auth check (team_id={team_id}): {error}");
            return false;
        }
    };
    let Some(team) = team else {
        return false;
    };
    let team_caller = TeamCaller {
        user_id: caller.user_id.clone(),
        user_role: caller.user_role,
        team_id: caller.team_id.clone(),
        team_service_account: caller.service_account,
    };
    match access.allows(&team_caller, &team, &TEAM_OR_ORG_ADMIN).await {
        Ok(allowed) => allowed,
        Err(error) => {
            tracing::debug!(
                "Org-admin check skipped during write-auth (team_id={team_id}): {error}"
            );
            false
        }
    }
}

// memory_endpoints.py _assert_write_access: visibility is not authority, so a mutation re-checks ownership
pub async fn assert_write_access(
    caller: &MemoryCaller,
    row: &MemoryRow,
    teams: &dyn TeamRows,
    access: &TeamAccess,
) -> Result<(), MemoryHttpError> {
    if is_admin(caller) {
        return Ok(());
    }
    let personal = match (row.user_id.as_deref(), caller.user_id.as_deref()) {
        (Some(owner), Some(caller_id)) => owner == caller_id,
        (Some(_), None) | (None, _) => false,
    };
    if personal {
        return Ok(());
    }
    let team_id = match (row.user_id.as_deref(), row.team_id.as_deref()) {
        (None, Some(team_id)) => team_id,
        (Some(_), _) | (None, None) => return Err(write_denied()),
    };
    if is_team_admin_for(teams, access, caller, team_id).await {
        return Ok(());
    }
    Err(write_denied())
}

// the row is visible but not the caller's to change
fn write_denied() -> MemoryHttpError {
    MemoryHttpError {
        status: StatusCode::FORBIDDEN,
        detail: "You do not have permission to modify this memory entry.".to_owned(),
    }
}
