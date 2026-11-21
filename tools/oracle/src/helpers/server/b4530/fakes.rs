use async_trait::async_trait;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_db::team_rows::team::TeamBase;
use betterlite_server::memory_records::access::TeamRows;
use betterlite_server::memory_records::models::MemoryRow;
use betterlite_server::team_access::access::OrgRoles;
use betterlite_server::team_access::error::TeamAccessError;

use super::inputs::RowInput;

// LiteLLM passes the real team repository; a case has no database, so every lookup fails the way a connection error does and the check must deny
pub(super) struct RefusingTeams;

#[async_trait]
impl TeamRows for RefusingTeams {
    async fn find_team(&self, _team_id: &str) -> Result<Option<TeamBase<String>>, TeamQueryError> {
        Err(TeamQueryError::from(DbError::NoDbConnected))
    }
}

// unreachable while every lookup fails, kept so the write check runs the production access type
pub(super) struct NoOrgRoles;

#[async_trait]
impl OrgRoles for NoOrgRoles {
    async fn is_org_admin(
        &self,
        _user_id: &str,
        _organization_id: &str,
    ) -> Result<bool, TeamAccessError> {
        Ok(false)
    }
}

// the row fields write authorization reads, with the fields it never looks at left empty
pub(super) fn case_row(input: &RowInput) -> MemoryRow {
    MemoryRow {
        memory_id: "case-memory".to_owned(),
        key: "case/key".to_owned(),
        value: String::new(),
        metadata: None,
        user_id: input.user_id.clone(),
        team_id: input.team_id.clone(),
        created_at: None,
        created_by: None,
        updated_at: None,
        updated_by: None,
    }
}
