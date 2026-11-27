use std::collections::BTreeMap;
use std::fmt;

use betterlite_db::resource_rows::project::ProjectTable;
use thiserror::Error;

use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_model_patterns::grant::key_model_aliases;
use crate::auth_prefetch::rows::CachedProject;
use crate::auth_request_models::RequestedModels;
use crate::auth_tenant_budgets::error::TenantBudgetError;
use crate::auth_tenant_budgets::{TenantAdmission, project};
use crate::entity_model_access::error::EntityModelAccessError;
use crate::entity_model_access::project::can_project_access_model;
use crate::resolved_key::auth::UserAPIKeyAuth;

// the project grant can invite either B886 directly or a test double that records the call
pub trait ProjectModelGrant: Send + Sync {
    fn check(
        &self,
        model: &RequestedModels,
        project_row: &ProjectTable,
        alias_targets: &BTreeMap<String, String>,
    ) -> Result<(), EntityModelAccessError>;
}

// B886's can_project_access_model with the request's router view and key aliases
pub struct DeploymentProjectGrant<'context> {
    pub context: ModelAccessContext<'context>,
}

impl fmt::Debug for DeploymentProjectGrant<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DeploymentProjectGrant")
    }
}

impl ProjectModelGrant for DeploymentProjectGrant<'_> {
    fn check(
        &self,
        model: &RequestedModels,
        project_row: &ProjectTable,
        alias_targets: &BTreeMap<String, String>,
    ) -> Result<(), EntityModelAccessError> {
        can_project_access_model(model, Some(project_row), alias_targets, &self.context)
    }
}

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("Project={project_id} is blocked. Update via `/project/update` if you're an admin.")]
    Blocked { project_id: String },
    #[error("{0}")]
    Grant(#[source] Box<EntityModelAccessError>),
    #[error("{0}")]
    Budget(#[source] Box<TenantBudgetError>),
}

// the collaborators the project checks call: the model grant and the tenant budget admission
pub struct ProjectCollaborators<'deps> {
    pub grant: &'deps dyn ProjectModelGrant,
    pub admission: TenantAdmission<'deps>,
}

// _run_project_checks: blocked first, then the model grant, then the hard budget, then the soft alert
pub async fn run_project_checks(
    project_row: Option<&CachedProject>,
    model: &RequestedModels,
    skip_budget_checks: bool,
    token: Option<&UserAPIKeyAuth>,
    collaborators: &ProjectCollaborators<'_>,
) -> Result<(), ProjectError> {
    let Some(checked) = project_row else {
        return Ok(());
    };
    let row = &checked.project;
    if row.blocked {
        return Err(ProjectError::Blocked {
            project_id: row.project_id.clone(),
        });
    }
    let requested = match model {
        RequestedModels::None => false,
        RequestedModels::One(name) => !name.is_empty(),
        RequestedModels::Many(names) => !names.is_empty(),
    };
    if requested && !row.models.is_empty() {
        let alias_targets = token.map_or_else(BTreeMap::new, |valid| {
            key_model_aliases(&valid.view.token.aliases.0)
        });
        collaborators
            .grant
            .check(model, row, &alias_targets)
            .map_err(|error| ProjectError::Grant(Box::new(error)))?;
    }
    if skip_budget_checks {
        return Ok(());
    }
    project::max_budget_check(checked, token, &collaborators.admission)
        .await
        .map_err(|error| ProjectError::Budget(Box::new(error)))?;
    let _soft = project::soft_budget_check(checked, token, collaborators.admission.alerts);
    Ok(())
}
