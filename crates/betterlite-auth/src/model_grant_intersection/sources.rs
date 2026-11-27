use std::collections::BTreeSet;
use std::error::Error;

use async_trait::async_trait;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;

use crate::auth_org_lookup::error::OrgLookupError;
use crate::auth_prefetch::rows::CachedProject;
use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_user_lookup::error::UserLookupError;
use crate::end_user_registry::errors::RegistryError;
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
