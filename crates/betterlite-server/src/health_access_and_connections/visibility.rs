use std::collections::BTreeSet;

use betterlite_auth::auth_identity::roles::effective_user_role;
use betterlite_auth::auth_model_lists::access_groups::AccessGroupScope;
use betterlite_auth::auth_model_lists::key_models::get_key_models;
use betterlite_auth::auth_model_patterns::ALL_PROXY_MODELS;
use betterlite_auth::auth_model_patterns::grant::resolve_key_models;
use betterlite_router::catalog_updates::catalog::CatalogHandle;
use betterlite_router::deployment_credentials::selection::deployment_usable_by_team;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::error::LookupError;
use betterlite_router::model_catalog_lookup::listing::should_include_deployment;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::model_group_lookups::access_groups::{AccessGroupMap, AccessGroups};
use betterlite_router::model_group_lookups::error::AccessGroupError;
use betterlite_router::routing_groups::index::RoutingGroupIndex;

use crate::auth::auth_identity::AuthContext;
use crate::health_probe_settings::targeting::deployments_targeted_by_name;
use crate::model_listing::ModelListingError;
use crate::model_listing::access::{KeyModelFields, model_names};
use crate::plugin_routes::routes::caller_identity;
use crate::team_access::caller::TeamCaller;

// _PROXY_ADMIN_ROLES: the full and the view-only admin both see provider routing fields
pub const PROXY_ADMIN_ROLES: [&str; 2] = ["proxy_admin", "proxy_admin_viewer"];

// user_api_key_dict and the key's model grants, as /health reads them: the role, the key's team and its allowlists
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HealthCaller {
    pub user_id: Option<String>,
    pub user_role: Option<String>,
    pub team_id: Option<String>,
    pub models: Vec<String>,
    pub team_models: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum CallerError {
    #[error("the caller identity cannot be read: {0}")]
    Identity(#[source] serde_json::Error),
    #[error("the key's model grants cannot be read: {0}")]
    KeyFields(#[source] ModelListingError),
}

impl HealthCaller {
    pub fn from_context(context: &AuthContext) -> Result<Self, CallerError> {
        let principal = &context.principal;
        let identity = caller_identity(principal).map_err(CallerError::Identity)?;
        let fields = match principal.source_key.0.as_ref() {
            Some(key) => KeyModelFields::from_key(key).map_err(CallerError::KeyFields)?,
            None => KeyModelFields::default(),
        };
        Ok(Self {
            user_id: identity.user_id,
            user_role: identity.user_role,
            team_id: fields.team_id,
            models: model_names(&fields.models),
            team_models: model_names(&fields.team_models),
        })
    }

    // the same caller as the model management checks read it
    #[must_use]
    pub fn team_caller(&self) -> TeamCaller {
        TeamCaller {
            user_id: self.user_id.clone(),
            user_role: Some(effective_user_role(self.user_role.as_deref())),
            team_id: self.team_id.clone(),
            team_service_account: false,
        }
    }
}

// _is_proxy_admin compares the raw role value, which may arrive as either the enum or its string
#[must_use]
pub fn is_proxy_admin(caller: &HealthCaller) -> bool {
    caller
        .user_role
        .as_deref()
        .is_some_and(|role| PROXY_ADMIN_ROLES.contains(&role))
}

#[derive(Debug, thiserror::Error)]
pub enum LookupFailure {
    #[error("the router lookup failed: {0}")]
    Lookup(#[from] LookupError),
    #[error("the model access groups could not be read: {0}")]
    AccessGroups(#[from] AccessGroupError),
}

// The router reads the health endpoints make: a deployment by id, the model list, the names and the access groups
pub trait HealthRouter: Send + Sync {
    fn get_deployment(&self, model_id: &str) -> Result<Option<DeploymentSpec>, LookupFailure>;
    fn get_model_list(
        &self,
        model_name: Option<&str>,
        team_id: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, LookupFailure>;
    fn get_model_names(&self, team_id: Option<&str>) -> Result<Vec<String>, LookupFailure>;
    fn get_model_access_groups(&self) -> Result<AccessGroupMap, LookupFailure>;
}

// A live router's catalog parts: the handle is read into a snapshot per lookup, so no borrowed row crosses an await
pub struct LiveRouter<'router> {
    pub catalog: &'router CatalogHandle,
    pub model_group_alias: &'router AliasMap,
    pub routing_groups: &'router RoutingGroupIndex,
    pub access_groups: &'router AccessGroups,
    pub inferred_provider: &'router (dyn Fn(&str) -> Option<String> + Send + Sync),
}

impl LiveRouter<'_> {
    #[cfg_attr(test, mutants::skip)]
    fn with_lookup<T>(&self, read: impl for<'read> FnOnce(CatalogLookup<'read>) -> T) -> T {
        let catalog = self.catalog.load_full();
        let inferred_provider = |model: &str| (self.inferred_provider)(model);
        let lookup = CatalogLookup {
            catalog: catalog.as_ref(),
            model_group_alias: self.model_group_alias,
            routing_groups: self.routing_groups,
            inferred_provider: &inferred_provider,
        };
        read(lookup)
    }
}

impl HealthRouter for LiveRouter<'_> {
    // each method is a CatalogLookup or AccessGroups read the router crate already tests; only the snapshot read is here
    #[cfg_attr(test, mutants::skip)]
    fn get_deployment(&self, model_id: &str) -> Result<Option<DeploymentSpec>, LookupFailure> {
        self.with_lookup(|lookup| -> Result<Option<DeploymentSpec>, LookupFailure> {
            let found = lookup.get_deployment(model_id)?;
            Ok(found.cloned())
        })
    }

    #[cfg_attr(test, mutants::skip)]
    fn get_model_list(
        &self,
        model_name: Option<&str>,
        team_id: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, LookupFailure> {
        self.with_lookup(|lookup| -> Result<Vec<DeploymentSpec>, LookupFailure> {
            Ok(lookup.get_model_list(model_name, team_id)?)
        })
    }

    #[cfg_attr(test, mutants::skip)]
    fn get_model_names(&self, team_id: Option<&str>) -> Result<Vec<String>, LookupFailure> {
        self.with_lookup(|lookup| -> Result<Vec<String>, LookupFailure> {
            Ok(lookup.get_model_names(team_id)?)
        })
    }

    #[cfg_attr(test, mutants::skip)]
    fn get_model_access_groups(&self) -> Result<AccessGroupMap, LookupFailure> {
        self.with_lookup(|lookup| -> Result<AccessGroupMap, LookupFailure> {
            Ok(self
                .access_groups
                .get_model_access_groups(&lookup, None, None, None)?)
        })
    }
}

// _health_accessible_model_names: None when the key is unrestricted, otherwise the names the caller may check
pub fn health_accessible_model_names(
    caller: &HealthCaller,
    router: Option<&dyn HealthRouter>,
) -> Result<Option<BTreeSet<String>>, LookupFailure> {
    let granted = resolve_key_models(
        &caller.models,
        &caller.team_models,
        caller.team_id.as_deref(),
    );
    if granted.is_empty()
        || granted
            .iter()
            .any(|model| model.as_str() == ALL_PROXY_MODELS)
    {
        return Ok(None);
    }
    let Some(router) = router else {
        return Ok(Some(granted.into_iter().collect()));
    };
    let names = router.get_model_names(caller.team_id.as_deref())?;
    let groups = router.get_model_access_groups()?;
    let expanded = get_key_models(
        &caller.models,
        &caller.team_models,
        &names,
        &groups,
        AccessGroupScope::default(),
    );
    Ok(Some(expanded.into_iter().collect()))
}

// _caller_may_probe_deployment: another team's deployment is never in scope, and the key's grants bound the names
#[must_use]
pub fn caller_may_probe_deployment(
    deployment: &DeploymentSpec,
    allowed_models: Option<&BTreeSet<String>>,
    router: Option<&dyn HealthRouter>,
    team_id: Option<&str>,
    caller_is_admin: bool,
) -> bool {
    if !caller_is_admin && !deployment_usable_by_team(deployment, team_id) {
        return false;
    }
    let Some(allowed) = allowed_models else {
        return true;
    };
    if router.is_none() {
        return allowed.contains(&deployment.model_name);
    }
    allowed
        .iter()
        .any(|name| should_include_deployment(name, deployment, team_id))
}

// _resolve_targeted_model_ids: model_id wins and matches model_info.id only; model targets the routed deployments
#[must_use]
pub fn resolve_targeted_model_ids(
    model_list: &[DeploymentSpec],
    model: Option<&str>,
    model_id: Option<&str>,
    team_id: Option<&str>,
) -> Option<BTreeSet<String>> {
    if let Some(requested) = model_id.filter(|id| !id.is_empty()) {
        return Some(
            model_list
                .iter()
                .filter_map(DeploymentSpec::id)
                .filter(|id| *id == requested)
                .map(str::to_owned)
                .collect(),
        );
    }
    let requested = model.filter(|name| !name.is_empty())?;
    Some(
        deployments_targeted_by_name(model_list, requested, team_id)
            .iter()
            .filter_map(|spec| spec.id().filter(|id| !id.is_empty()))
            .map(str::to_owned)
            .collect(),
    )
}
