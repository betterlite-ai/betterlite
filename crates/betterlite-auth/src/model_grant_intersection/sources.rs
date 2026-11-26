use std::collections::BTreeSet;
use std::error::Error;

use async_trait::async_trait;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;

use crate::auth_org_lookup::error::OrgLookupError;
use crate::auth_org_lookup::organization::{OrgId, OrgLookup, get_org_object};
use crate::auth_org_lookup::project::get_project_object;
use crate::auth_prefetch::rows::CachedProject;
use crate::auth_tag_budgets::cache::TagBudgetCache;
use crate::auth_tag_budgets::registry::{RegistryKind, load_registry};
use crate::auth_tag_budgets::rows::{BudgetRows, ModelAccessGroupBudgetRow};
use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_team_lookup::lookup::{TeamRequest, TeamSource};
use crate::auth_user_lookup::error::UserLookupError;
use crate::auth_user_lookup::lookup::UserLookupRequest;
use crate::end_user_registry::errors::RegistryError;
use crate::identity_grants::loaders::{MembershipLoader, TeamLoader, UserLoader};
use crate::membership_load::error::MembershipLoadError;

// The cache, database and registry reads the walks make, injected so a caller can script them.
#[async_trait]
pub trait GrantSources: Send + Sync {
    async fn team_object(
        &self,
        team_id: &str,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError>;
    async fn user_object(&self, user_id: &str)
    -> Result<Option<LiteLLMUserTable>, UserLookupError>;
    async fn project_object(
        &self,
        project_id: &str,
        now: f64,
    ) -> Result<Option<CachedProject>, OrgLookupError>;
    async fn organization(&self, org_id: &str)
    -> Result<Option<OrganizationTable>, OrgLookupError>;
    async fn membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError>;
    async fn budget_groups(&self) -> Result<Option<BTreeSet<String>>, RegistryError>;
}

// The router's answer for one requested model, behind a trait Betterlite-auth can name without the router crate.
#[async_trait]
pub trait ServingGroups: Send + Sync {
    async fn groups_for(
        &self,
        model: &str,
        team_id: Option<&str>,
    ) -> Result<BTreeSet<String>, ServingFailure>;
}

// A router failure crossing crates as a cause, since betterlite-auth cannot name the router's error enum.
#[derive(Debug, thiserror::Error)]
#[error("the model access group lookup failed")]
pub struct ServingFailure(#[source] Box<dyn Error + Send + Sync>);

impl From<Box<dyn Error + Send + Sync>> for ServingFailure {
    fn from(cause: Box<dyn Error + Send + Sync>) -> Self {
        Self(cause)
    }
}

// The production sources: each load goes through the crate that owns the lookup.
pub struct LiveSources<'sources> {
    pub teams: &'sources dyn TeamLoader,
    pub users: &'sources dyn UserLoader,
    pub memberships: &'sources dyn MembershipLoader,
    pub organizations: &'sources OrgLookup,
    pub budgets: &'sources dyn TagBudgetCache,
    pub budget_rows: &'sources dyn BudgetRows<ModelAccessGroupBudgetRow>,
}

#[async_trait]
impl GrantSources for LiveSources<'_> {
    async fn team_object(
        &self,
        team_id: &str,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
        let request = TeamRequest {
            team_id,
            source: TeamSource::CacheThenDatabase,
            team_id_upsert: false,
        };
        self.teams.load_team(request).await
    }

    async fn user_object(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, UserLookupError> {
        let request = UserLookupRequest {
            user_id: Some(user_id),
            user_id_upsert: false,
            ..UserLookupRequest::default()
        };
        self.users.load_user(request).await
    }

    async fn project_object(
        &self,
        project_id: &str,
        now: f64,
    ) -> Result<Option<CachedProject>, OrgLookupError> {
        get_project_object(self.organizations, project_id, now).await
    }

    async fn organization(
        &self,
        org_id: &str,
    ) -> Result<Option<OrganizationTable>, OrgLookupError> {
        get_org_object(self.organizations, OrgId::Text(org_id.to_owned()), false).await
    }

    async fn membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        self.memberships.load_membership(user_id, team_id).await
    }

    async fn budget_groups(&self) -> Result<Option<BTreeSet<String>>, RegistryError> {
        load_registry(RegistryKind::Group, self.budgets, self.budget_rows).await
    }
}
