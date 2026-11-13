use std::slice::from_ref;

use crate::deployment_catalog::error::CatalogLookupError;
use crate::deployment_identity::spec::DeploymentSpec;

// The policy layers the server owns. Each answer is for one authenticated caller, never read from request metadata.
pub trait CandidateView {
    // Router._get_routing_group_deployments
    fn routing_group(&self, model: &str, team_id: Option<&str>) -> Option<Vec<DeploymentSpec>>;

    // Router._filter_deployments_by_model_access_groups
    fn filter_access_groups(
        &self,
        model: &str,
        deployments: Vec<DeploymentSpec>,
        team_id: Option<&str>,
    ) -> Vec<DeploymentSpec>;

    // the reservation filter of Router._filter_reserved_deployments, which sees the deployments after the strategy markers are dropped
    fn filter_reserved(
        &self,
        deployments: Vec<DeploymentSpec>,
        team_id: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, CatalogLookupError>;

    // the first entry of the "*" fallback chain
    fn default_fallback(&self) -> Option<String>;

    // inferred_provider, which the wildcard lookup tries as a prefix after the bare name
    fn inferred_provider(&self, model: &str) -> Option<String>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Authority<'caller> {
    pub team_id: Option<&'caller str>,
    pub proxy_admin: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct LookupRequest<'request> {
    pub model: &'request str,
    pub specific_deployment: bool,
    pub authority: Authority<'request>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AliasSource {
    DeploymentId,
    ModelGroupAlias,
    ModelAliasMap,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AliasStep {
    pub source: AliasSource,
    pub from: String,
    pub to: String,
}

// A deployment id names one deployment; everything else names a pool to select from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Candidates {
    Single(Box<DeploymentSpec>),
    Pool(Vec<DeploymentSpec>),
}

impl Candidates {
    #[must_use]
    pub fn deployments(&self) -> &[DeploymentSpec] {
        match self {
            Self::Single(deployment) => from_ref(deployment.as_ref()),
            Self::Pool(pool) => pool,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogMatch {
    pub group: String,
    pub candidates: Candidates,
    pub aliases: Vec<AliasStep>,
}
