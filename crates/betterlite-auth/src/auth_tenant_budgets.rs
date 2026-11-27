pub mod access_groups;
pub mod alerts;
pub mod denial;
pub mod error;
pub mod organization;
pub mod project;
pub mod tags;
mod tests;

use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use futures_util::future::join3;

use crate::auth_key_budgets::spend::SpendReader;
use crate::auth_org_lookup::organization::OrgLookup;
use crate::auth_prefetch::rows::CachedProject;
use crate::auth_tag_budgets::cache::TagBudgetCache;
use crate::auth_tag_budgets::rows::{BudgetRows, ModelAccessGroupBudgetRow};
use crate::resolved_key::auth::UserAPIKeyAuth;

use self::alerts::TenantAlertSink;
use self::error::TenantBudgetError;

// the collaborators the proxy wires: the cross-pod reader and the alert sink the checks hand records to
pub struct TenantAdmission<'deps> {
    pub reader: &'deps dyn SpendReader,
    pub alerts: Option<&'deps dyn TenantAlertSink>,
}

// the tenant objects and batch reads the checks share; an absent collaborator means the proxy cannot read it
pub struct TenantSources<'sources> {
    pub database: bool,
    pub organizations: Option<&'sources OrgLookup>,
    pub cache: Option<&'sources dyn TagBudgetCache>,
    pub tags: Option<&'sources dyn BudgetRows<TagTable>>,
    pub groups: Option<&'sources dyn BudgetRows<ModelAccessGroupBudgetRow>>,
}

// what the request resolved before this module runs: the key, its team, the project it charges and its tags
pub struct TenantRequest<'request> {
    pub token: Option<&'request mut UserAPIKeyAuth>,
    pub team: Option<&'request LiteLLMTeamTable>,
    pub project: Option<&'request CachedProject>,
    pub tags: &'request [String],
    pub matched_model_access_groups: &'request [String],
}

// common_checks runs the project checks first, then settles the organization, tag and access-group checks in order
pub async fn admit(
    request: &mut TenantRequest<'_>,
    sources: &TenantSources<'_>,
    admission: &TenantAdmission<'_>,
) -> Result<(), TenantBudgetError> {
    if let Some(project) = request.project {
        project::max_budget_check(project, request.token.as_deref(), admission).await?;
        let _soft = project::soft_budget_check(project, request.token.as_deref(), admission.alerts);
    }
    let organization = organization::max_budget_check(
        request.token.as_deref_mut(),
        request.team,
        sources.organizations,
        admission,
    );
    let tags = tags::max_budget_check(request.tags, sources, admission);
    let groups = access_groups::access_group_max_budget_check(
        request.matched_model_access_groups,
        sources,
        admission,
    );
    let (organization_check, tags_check, groups_check) = join3(organization, tags, groups).await;
    organization_check.and(tags_check).and(groups_check)
}
