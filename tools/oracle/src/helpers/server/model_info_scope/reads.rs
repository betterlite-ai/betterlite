use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_router::model_group_lookups::access_groups::AccessGroupMap;
use betterlite_server::model_info_scope::compose::{
    ModelCreator, ScopeFuture, ScopeReads, TeamSelection, UserScope,
};
use betterlite_server::model_info_scope::error::ScopeError;
use betterlite_server::model_info_scope::teams::TeamScope;

use super::records::Input;

#[derive(Default)]
struct CaseReads {
    creators: BTreeMap<String, Option<String>>,
    user_id: Option<String>,
    user: Option<UserScope>,
    teams: Vec<TeamScope>,
    groups: AccessGroupMap,
}

impl CaseReads {
    fn stored_user(&self, user_id: &str) -> Option<UserScope> {
        self.user
            .as_ref()
            .filter(|_stored| self.user_id.as_deref() == Some(user_id))
            .cloned()
    }
}

impl ScopeReads for CaseReads {
    fn model_creator<'scope>(
        &'scope self,
        model_id: &'scope str,
    ) -> ScopeFuture<'scope, Result<Option<ModelCreator>, ScopeError>> {
        Box::pin(async move {
            Ok(self.creators.get(model_id).map(|created_by| ModelCreator {
                created_by: created_by.clone(),
            }))
        })
    }

    fn user_scope<'scope>(
        &'scope self,
        user_id: &'scope str,
    ) -> ScopeFuture<'scope, Result<Option<UserScope>, ScopeError>> {
        Box::pin(async move { Ok(self.stored_user(user_id)) })
    }

    fn team_scopes<'scope>(
        &'scope self,
        _selection: &'scope TeamSelection,
    ) -> ScopeFuture<'scope, Result<Vec<TeamScope>, ScopeError>> {
        Box::pin(async move { Ok(self.teams.clone()) })
    }

    fn access_group_models<'scope>(
        &'scope self,
        _access_group_ids: &'scope [String],
    ) -> ScopeFuture<'scope, Result<AccessGroupMap, ScopeError>> {
        Box::pin(async move { Ok(self.groups.clone()) })
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
