use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::body::{Body, Bytes};
use axum::http::Request;
use betterlite_auth::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use betterlite_auth::auth_key_budgets::key_budget::Denial;
use betterlite_auth::auth_key_budgets::spend::{SpendReadError, SpendReader};
use betterlite_auth::auth_org_lookup::organization::OrgLookup;
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_tag_budgets::cache::TagBudgetCache;
use betterlite_auth::auth_tag_budgets::rows::{BudgetRows, ModelAccessGroupBudgetRow};
use betterlite_auth::auth_tenant_budgets::alerts::TenantAlertSink;
use betterlite_auth::auth_tenant_budgets::error::TenantBudgetError;
use betterlite_auth::auth_tenant_budgets::{TenantAdmission, TenantRequest, TenantSources, admit};
use betterlite_auth::current_spend::SpendQuery;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_destination::parameters::tags_from_request_body;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use rust_decimal::Decimal;

use crate::auth::auth_identity::AuthContext;
use crate::auth::current_spend::spend_failure_exception;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

// what the auth service resolved before this stage: the caller, its team and project, the groups it matched, and the
// collaborators the four checks read
#[derive(Default)]
pub struct TenantBudgetRule {
    pub auth: Option<UserAPIKeyAuth>,
    pub team: Option<LiteLLMTeamTable>,
    pub project: Option<CachedProject>,
    pub matched_model_access_groups: Vec<String>,
    pub organizations: Option<Arc<OrgLookup>>,
    pub cache: Option<Arc<dyn TagBudgetCache>>,
    pub tags: Option<Arc<dyn BudgetRows<TagTable>>>,
    pub groups: Option<Arc<dyn BudgetRows<ModelAccessGroupBudgetRow>>>,
    pub reader: Option<Arc<dyn SpendReader>>,
    pub alerts: Option<Arc<dyn TenantAlertSink>>,
    pub applied: Mutex<Option<UserAPIKeyAuth>>,
}

// the token never reaches the debug line, only whether the stage has one
impl fmt::Debug for TenantBudgetRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TenantBudgetRule")
            .field("has_auth", &self.auth.is_some())
            .field("has_team", &self.team.is_some())
            .field(
                "project_id",
                &self.project.as_ref().map(|row| &row.project.project_id),
            )
            .field(
                "matched_model_access_groups",
                &self.matched_model_access_groups,
            )
            .field("has_organizations", &self.organizations.is_some())
            .field("has_tags", &self.tags.is_some())
            .field("has_groups", &self.groups.is_some())
            .field("has_reader", &self.reader.is_some())
            .field("has_alerts", &self.alerts.is_some())
            .field(
                "has_applied",
                &self.applied.try_lock().is_ok_and(|held| held.is_some()),
            )
            .finish_non_exhaustive()
    }
}

impl TenantBudgetRule {
    // the key after the checks, which carries the organization alias and budget snapshot they resolved
    #[must_use]
    pub fn applied(&self) -> Option<UserAPIKeyAuth> {
        self.applied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Debug)]
pub struct TenantBudgetStageInput {
    pub context: AuthContext,
    pub rule: Arc<TenantBudgetRule>,
    // B1102 reads the tags the request body carries, metadata first and duplicates kept
    pub tags: Vec<String>,
    pub database: bool,
}

// the authentication service inserts the rule once it resolved the key; without one the stage is inert
pub(crate) fn rule_for_request(request: &Request<Body>) -> Arc<TenantBudgetRule> {
    request
        .extensions()
        .get::<Arc<TenantBudgetRule>>()
        .map_or_else(|| Arc::new(TenantBudgetRule::default()), Arc::clone)
}

fn database_for_request(request: &Request<Body>) -> bool {
    request
        .extensions()
        .get::<Arc<ServerState>>()
        .is_some_and(|state| state.auth_tenant_budgets.database)
}

fn tags_for_body(body: Option<&Bytes>) -> Vec<String> {
    let Some(data) = body.and_then(|bytes| serde_json::from_slice::<RequestData>(bytes).ok())
    else {
        return Vec::new();
    };
    tags_from_request_body(&data)
}

// the request body supplies the tags, so the stage input is built while the caller still owns it
pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
    body: Option<&Bytes>,
) -> Result<TenantBudgetStageInput, AuthenticationFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    Ok(TenantBudgetStageInput {
        context,
        rule: rule_for_request(request),
        tags: tags_for_body(body),
        database: database_for_request(request),
    })
}

// common_checks: the project checks first, then the organization, tag and access-group checks settle together
pub(crate) async fn prefetch(stage: &TenantBudgetStageInput) -> Result<(), Box<ProxyException>> {
    let rule = &stage.rule;
    let mut checked = rule.auth.clone();
    let unconfigured = UnconfiguredReader;
    let admission = TenantAdmission {
        reader: rule.reader.as_deref().unwrap_or(&unconfigured),
        alerts: rule.alerts.as_deref(),
    };
    let sources = TenantSources {
        database: stage.database,
        organizations: rule.organizations.as_deref(),
        cache: rule.cache.as_deref(),
        tags: rule.tags.as_deref(),
        groups: rule.groups.as_deref(),
    };
    let mut request = TenantRequest {
        token: checked.as_mut(),
        team: rule.team.as_ref(),
        project: rule.project.as_ref(),
        tags: &stage.tags,
        matched_model_access_groups: &rule.matched_model_access_groups,
    };
    admit(&mut request, &sources, &admission)
        .await
        .map_err(|error| Box::new(tenant_budget_exception(&error)))?;
    if let Some(checked) = checked {
        *rule.applied.lock().unwrap_or_else(PoisonError::into_inner) = Some(checked);
    }
    Ok(())
}

// BudgetExceededError keeps its configured status; an unverifiable counter is the 503; a failed batch read is a 401
fn tenant_budget_exception(error: &TenantBudgetError) -> ProxyException {
    match error {
        TenantBudgetError::Denied(denial) => budget_exceeded_exception(denial),
        TenantBudgetError::Spend(failure) => spend_failure_exception(failure),
        TenantBudgetError::Tag(_) => authentication_exception(&error.to_string()),
    }
}

fn budget_exceeded_exception(denial: &Denial) -> ProxyException {
    let failure = AuthFailure::BudgetExceeded {
        message: denial.message.clone(),
        status_code: Some(denial.status_code),
        llm_provider: String::new(),
    };
    as_proxy_exception(&failure).exception().clone()
}

fn authentication_exception(message: &str) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: format!("Authentication Error, {message}"),
        error_type: "auth_error".to_owned(),
        param: Some("None".to_owned()),
        code: ExceptionCode::Status(401_i64),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

// a rule with no reader cannot verify spend, which raises the 503; a rule with nothing priced never reads
struct UnconfiguredReader;

#[async_trait]
impl SpendReader for UnconfiguredReader {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        Err(SpendReadError::Unverifiable {
            counter_key: query.counter_key.to_owned(),
        })
    }
}
