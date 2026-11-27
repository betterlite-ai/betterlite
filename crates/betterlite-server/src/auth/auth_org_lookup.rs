use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::Request;
use betterlite_auth::auth_org_lookup::error::{OrgLookupError, proxy_exception};
use betterlite_auth::auth_org_lookup::last_known::get_org_object_for_request;
use betterlite_auth::auth_org_lookup::organization::OrgLookup;
use betterlite_db::database_availability::policy::AllowRequestsOnOutage;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_error::proxy_exception::ProxyException;

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

// What a request knows about its organization: the id the identity resolved and the lookup to run.
#[derive(Clone, Default)]
pub struct OrgRule {
    pub org_id: Option<String>,
    pub lookup: Option<Arc<dyn OrgLookupForRequest>>,
}

// the collaborator stays out of the line, since it carries the cache and the database handles
impl fmt::Debug for OrgRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OrgRule")
            .field("org_id", &self.org_id)
            .finish_non_exhaustive()
    }
}

// LiteLLM's get_org_object_for_request, over whatever cache and database the proxy wired.
#[async_trait]
pub trait OrgLookupForRequest: Send + Sync {
    async fn org_object(
        &self,
        org_id: &str,
        policy: &AllowRequestsOnOutage,
    ) -> Result<Option<OrganizationTable>, OrgLookupError>;
}

// the production lookup is the request-scoped domain function itself
#[async_trait]
impl OrgLookupForRequest for OrgLookup {
    async fn org_object(
        &self,
        org_id: &str,
        policy: &AllowRequestsOnOutage,
    ) -> Result<Option<OrganizationTable>, OrgLookupError> {
        get_org_object_for_request(self, org_id, policy).await
    }
}

#[derive(Clone, Debug)]
pub struct OrgStageInput {
    pub context: AuthContext,
    pub rule: Arc<OrgRule>,
    pub policy: AllowRequestsOnOutage,
    pub database: bool,
}

fn rule_for_request(request: &Request<Body>) -> Arc<OrgRule> {
    request
        .extensions()
        .get::<Arc<OrgRule>>()
        .map_or_else(|| Arc::new(OrgRule::default()), Arc::clone)
}

// the lookup is asynchronous, so the stage keeps its own input and the caller awaits the prefetch
pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> Result<OrgStageInput, AuthenticationFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let (database, policy) = request.extensions().get::<Arc<ServerState>>().map_or_else(
        || (false, AllowRequestsOnOutage::default()),
        |state| {
            (
                state.auth_org_lookup.database,
                state.auth_org_lookup.allow_requests_on_outage.clone(),
            )
        },
    );
    Ok(OrgStageInput {
        context,
        rule: rule_for_request(request),
        policy,
        database,
    })
}

pub(crate) async fn prefetched(stage: Box<OrgStageInput>) -> Result<StageInput, StageFailure> {
    prefetch(&stage).await.map_err(StageFailure::OrgLookup)?;
    Ok(StageInput::AuthOrgLookup(stage))
}

// LiteLLM's _inherit_org_identity reads nothing without a database, an org id or a wired lookup
pub(crate) async fn prefetch(stage: &OrgStageInput) -> Result<(), Box<ProxyException>> {
    let Some(lookup) = stage.rule.lookup.as_deref() else {
        return Ok(());
    };
    let Some(org_id) = stage.rule.org_id.as_deref() else {
        return Ok(());
    };
    if !stage.database {
        return Ok(());
    }
    match lookup.org_object(org_id, &stage.policy).await {
        Ok(_found) => Ok(()),
        Err(error) => Err(Box::new(proxy_exception(&error))),
    }
}
