use async_trait::async_trait;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_server::model_team_scope::reads::{TeamScopeReadError, TeamScopeReads};

use super::records::Input;

// The one stored user row a case declares; the scope reads nothing else.
#[derive(Default)]
pub(super) struct CaseReads {
    user_id: Option<String>,
    teams: Option<Vec<String>>,
}

#[async_trait]
impl TeamScopeReads for CaseReads {
    async fn user_teams(&self, user_id: &str) -> Result<Option<Vec<String>>, TeamScopeReadError> {
        if self.user_id.as_deref() == Some(user_id) {
            return Ok(self.teams.clone());
        }
        Ok(None)
    }

    async fn team_models(&self, _team_id: &str) -> Result<Option<Vec<String>>, TeamScopeReadError> {
        Ok(None)
    }

    async fn model_ids_by_names(
        &self,
        _names: &[String],
    ) -> Result<Vec<String>, TeamScopeReadError> {
        Ok(Vec::new())
    }

    async fn model_row(
        &self,
        _model_id: &str,
    ) -> Result<Option<ProxyModelTable>, TeamScopeReadError> {
        Ok(None)
    }
}

pub(super) fn reads(input: &Input) -> CaseReads {
    let user_id = input.user.as_ref().map(|record| record.user_id.clone());
    let teams = input.user.as_ref().map(|record| record.teams.clone());
    CaseReads { user_id, teams }
}
