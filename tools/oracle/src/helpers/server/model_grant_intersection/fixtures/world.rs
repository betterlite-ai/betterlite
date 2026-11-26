use std::collections::{BTreeMap, BTreeSet};
use std::io::Error as IoError;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_org_lookup::error::{OrgFailure, OrgLookupError};
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_team_lookup::error::{TeamFailure, TeamLookupError};
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::end_user_registry::errors::RegistryError;
use betterlite_auth::membership_load::error::MembershipLoadError;
use betterlite_auth::model_grant_intersection::sources::{
    GrantSources, ServingFailure, ServingGroups,
};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_oracle_helper_api::error::HelperError;
use serde_json::Error as JsonError;

use super::input::{self, Fault, Input};

// The scripted cache, database and router the production walks read, plus the calls they made.
pub(crate) struct Fixture {
    pub(crate) team: Option<LiteLLMTeamTableCachedObj>,
    user: Option<LiteLLMUserTable>,
    pub(crate) project: Option<CachedProject>,
    org: Option<OrganizationTable>,
    pub(crate) membership: Option<LiteLLMTeamMembership>,
    registry: Option<BTreeSet<String>>,
    serving: BTreeMap<String, BTreeSet<String>>,
    outages: Vec<Fault>,
    events: Mutex<Vec<String>>,
}

impl Fixture {
    pub(crate) fn build(input: &Input) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            team: input.team.as_ref().map(input::team_row).transpose()?,
            user: input.user.as_ref().map(input::user_row).transpose()?,
            project: input.project.as_ref().map(input::project_row).transpose()?,
            org: input.org.as_ref().map(input::org_row).transpose()?,
            membership: input
                .membership
                .as_ref()
                .map(input::membership_row)
                .transpose()?,
            registry: input
                .registry
                .as_ref()
                .map(|names| names.iter().cloned().collect()),
            serving: input
                .serving
                .iter()
                .map(|(model, groups)| (model.clone(), groups.iter().cloned().collect()))
                .collect(),
            outages: input.outages.clone(),
            events: Mutex::new(Vec::new()),
        })
    }

    #[must_use]
    pub(crate) fn events(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, event: String) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

#[async_trait]
impl GrantSources for Fixture {
    async fn team_object(
        &self,
        team_id: &str,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
        self.record(format!("read team {team_id}"));
        if self.outages.contains(&Fault::Team) {
            return Err(TeamLookupError::TeamUnavailable {
                team_id: team_id.to_owned(),
                cause: Box::new(TeamFailure::Cadence(JsonError::io(IoError::other(
                    "the team table is down",
                )))),
            });
        }
        match self.team.as_ref() {
            Some(row) if row.team.fields.base.team_id == team_id => Ok(row.clone()),
            _ => Err(TeamLookupError::TeamNotFound {
                team_id: team_id.to_owned(),
            }),
        }
    }

    async fn user_object(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, UserLookupError> {
        self.record(format!("read user {user_id}"));
        if self.outages.contains(&Fault::User) {
            return Err(UserLookupError::Unavailable(Box::new(IoError::other(
                "the user table is down",
            ))));
        }
        Ok(self
            .user
            .as_ref()
            .filter(|row| row.user_id == user_id)
            .cloned())
    }

    async fn project_object(
        &self,
        project_id: &str,
        _now: f64,
    ) -> Result<Option<CachedProject>, OrgLookupError> {
        self.record(format!("read project {project_id}"));
        if self.outages.contains(&Fault::Project) {
            return Err(table_failure("the project table is down"));
        }
        Ok(self
            .project
            .as_ref()
            .filter(|row| row.project.project_id == project_id)
            .cloned())
    }

    async fn organization(
        &self,
        org_id: &str,
    ) -> Result<Option<OrganizationTable>, OrgLookupError> {
        self.record(format!("read org {org_id}"));
        if self.outages.contains(&Fault::Org) {
            return Err(table_failure("the organization table is down"));
        }
        Ok(self
            .org
            .as_ref()
            .filter(|row| row.organization_id.as_deref() == Some(org_id))
            .cloned())
    }

    async fn membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        self.record(format!("read membership {user_id} {team_id}"));
        if self.outages.contains(&Fault::Membership) {
            return Err(MembershipLoadError::NoDatabase);
        }
        Ok(self
            .membership
            .as_ref()
            .filter(|row| row.user_id == user_id && row.team_id == team_id)
            .cloned())
    }

    async fn budget_groups(&self) -> Result<Option<BTreeSet<String>>, RegistryError> {
        self.record("read registry".to_owned());
        Ok(self.registry.clone())
    }
}

#[async_trait]
impl ServingGroups for Fixture {
    async fn groups_for(
        &self,
        model: &str,
        team_id: Option<&str>,
    ) -> Result<BTreeSet<String>, ServingFailure> {
        self.record(format!("serving {model} team={}", team_id.unwrap_or("-")));
        if self.outages.contains(&Fault::Serving) {
            return Err(ServingFailure::from(Box::new(IoError::other(
                "the access group store is down",
            ))));
        }
        Ok(self.serving.get(model).cloned().unwrap_or_default())
    }
}

fn table_failure(message: &str) -> OrgLookupError {
    OrgLookupError::Failure(OrgFailure::Row(Box::new(JsonError::io(IoError::other(
        message,
    )))))
}
