use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_server::model_info_scope::compose::{
    GroupModels, ModelCreator, ScopeReads, TeamSelection, UserScope,
};
use betterlite_server::model_info_scope::error::ScopeError;
use betterlite_server::model_info_scope::teams::TeamScope;

use super::records::Input;

#[derive(Default)]
pub(super) struct CaseReads {
    creators: BTreeMap<String, Option<String>>,
    user_id: Option<String>,
    user: Option<UserScope>,
    teams: Vec<TeamScope>,
    groups: GroupModels,
}

impl CaseReads {
    fn stored_user(&self, user_id: &str) -> Option<UserScope> {
        self.user
            .as_ref()
            .filter(|_stored| self.user_id.as_deref() == Some(user_id))
            .cloned()
    }
}

#[async_trait]
impl ScopeReads for CaseReads {
    async fn model_creator(&self, model_id: &str) -> Result<Option<ModelCreator>, ScopeError> {
        Ok(self.creators.get(model_id).map(|created_by| ModelCreator {
            created_by: created_by.clone(),
        }))
    }

    async fn user_scope(&self, user_id: &str) -> Result<Option<UserScope>, ScopeError> {
        Ok(self.stored_user(user_id))
    }

    async fn team_scopes(&self, _selection: &TeamSelection) -> Result<Vec<TeamScope>, ScopeError> {
        Ok(self.teams.clone())
    }

    async fn access_group_models(
        &self,
        _access_group_ids: &[String],
    ) -> Result<GroupModels, ScopeError> {
        Ok(self.groups.clone())
    }
}

pub(super) fn reads(input: &Input) -> CaseReads {
    let user = input.user.as_ref().map(|record| UserScope {
        teams: record.teams.clone(),
        models: record
            .models
            .iter()
            .map(|name| JsonData::String(name.clone()))
            .collect(),
    });
    CaseReads {
        creators: input.creators.clone(),
        user_id: input.user.as_ref().map(|record| record.user_id.clone()),
        user,
        teams: input
            .teams
            .iter()
            .map(|record| TeamScope {
                team_id: record.team_id.clone(),
                models: record.models.clone(),
                access_group_ids: record.access_group_ids.clone().unwrap_or_default(),
            })
            .collect(),
        groups: input
            .access_groups
            .iter()
            .map(|record| {
                (
                    record.access_group_id.clone(),
                    record.access_model_names.clone(),
                )
            })
            .collect(),
    }
}
