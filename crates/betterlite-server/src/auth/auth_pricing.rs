use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{Body, Bytes};
use axum::http::Request;
use betterlite_auth::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use betterlite_auth::auth_key_budgets::key_budget::Denial;
use betterlite_auth::auth_key_budgets::money::Money;
use betterlite_auth::auth_key_budgets::spend::{SpendReadError, SpendReader};
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::{
    ProviderInference, UnresolvedProviderInference,
};
use betterlite_auth::auth_model_patterns::view::ModelAccessView;
use betterlite_auth::auth_org_lookup::error::proxy_exception as org_lookup_exception;
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_pricing::catalog::PricingCatalog;
use betterlite_auth::auth_pricing::project::{
    DeploymentProjectGrant, ProjectError, run_project_checks,
};
use betterlite_auth::auth_pricing::request::{
    RequestFacts, enforce_user_param_check, global_proxy_budget_check,
    reject_clientside_metadata_tags_check,
};
use betterlite_auth::auth_pricing::routes::request_skips_budget_checks;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::auth_tenant_budgets::TenantAdmission;
use betterlite_auth::auth_tenant_budgets::alerts::TenantAlertSink;
use betterlite_auth::auth_tenant_budgets::error::TenantBudgetError;
use betterlite_auth::current_spend::SpendQuery;
use betterlite_auth::entity_model_access::error::EntityModelAccessError;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::auth_pricing::{MaxBudget, Settings};
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::auth::auth_model_patterns::max_recurse_depth_for_request;
use crate::auth::current_spend::spend_failure_exception;
use crate::auth::team_budget_admission::team_budget_exception;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

static UNRESOLVED_PROVIDER: UnresolvedProviderInference = UnresolvedProviderInference;

// what the auth service resolved before this stage: the caller, its project, the request model and the pricing collaborators
pub struct PricingRule {
    pub auth: Option<UserAPIKeyAuth>,
    pub project: Option<CachedProject>,
    pub model: RequestedModels,
    pub catalog: Option<Arc<dyn PricingCatalog>>,
    pub view: Option<Arc<dyn ModelAccessView>>,
    pub provider: Option<Arc<dyn ProviderInference>>,
    pub global_model_aliases: BTreeMap<String, String>,
    pub reader: Option<Arc<dyn SpendReader>>,
    pub alerts: Option<Arc<dyn TenantAlertSink>>,
    pub global_spend: Option<Decimal>,
}

impl Default for PricingRule {
    fn default() -> Self {
        Self {
            auth: None,
            project: None,
            model: RequestedModels::None,
            catalog: None,
            view: None,
            provider: None,
            global_model_aliases: BTreeMap::new(),
            reader: None,
            alerts: None,
            global_spend: None,
        }
    }
}

// the token never reaches the debug line, only whether the stage has one
impl fmt::Debug for PricingRule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PricingRule")
            .field("has_auth", &self.auth.is_some())
            .field(
                "project_id",
                &self.project.as_ref().map(|row| &row.project.project_id),
            )
            .field("model", &self.model)
            .field("has_catalog", &self.catalog.is_some())
            .field("has_reader", &self.reader.is_some())
            .field("global_spend", &self.global_spend)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct PricingStageInput {
    pub context: AuthContext,
    pub rule: Arc<PricingRule>,
    pub method: Option<String>,
    pub route: String,
    pub body: JsonObject,
    pub settings: Settings,
    pub max_recurse_depth: i64,
}

// the authentication service inserts the rule once it resolved the caller; without one the stage is inert
pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
    body: Option<&Bytes>,
) -> Result<StageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let rule = request
        .extensions()
        .get::<Arc<PricingRule>>()
        .map_or_else(|| Arc::new(PricingRule::default()), Arc::clone);
    let settings = request
        .extensions()
        .get::<Arc<ServerState>>()
        .map_or_else(Settings::default, |state| *state.auth_pricing.settings);
    Ok(StageInput::AuthPricing(Box::new(PricingStageInput {
        context,
        rule,
        method: Some(request.method().as_str().to_owned()),
        route: request.uri().path().to_owned(),
        body: body
            .and_then(|bytes| serde_json::from_slice::<JsonObject>(bytes).ok())
            .unwrap_or_default(),
        settings,
        max_recurse_depth: max_recurse_depth_for_request(request),
    })))
}

pub(crate) async fn prefetched(stage: Box<PricingStageInput>) -> Result<StageInput, StageFailure> {
    prefetch(&stage).await.map_err(StageFailure::AuthPricing)?;
    Ok(StageInput::AuthPricing(stage))
}

// common_checks' project checks, then the client tag refusal, the user requirement and the global ceiling
pub(crate) async fn prefetch(stage: &PricingStageInput) -> Result<(), Box<ProxyException>> {
    let rule = &stage.rule;
    let facts = RequestFacts {
        method: stage.method.as_deref(),
        route: &stage.route,
        body: &stage.body,
    };
    let skip = request_skips_budget_checks(&stage.route, &rule.model, rule.catalog.as_deref());
    let context = ModelAccessContext {
        view: rule.view.as_deref(),
        global_model_aliases: &rule.global_model_aliases,
        compaction: None,
        provider: rule.provider.as_deref().unwrap_or(&UNRESOLVED_PROVIDER),
        max_recurse_depth: stage.max_recurse_depth,
    };
    let grant = DeploymentProjectGrant { context };
    let unconfigured = UnconfiguredReader;
    let admission = TenantAdmission {
        reader: rule.reader.as_deref().unwrap_or(&unconfigured),
        alerts: rule.alerts.as_deref(),
    };
    run_project_checks(
        rule.project.as_ref(),
        &rule.model,
        skip,
        rule.auth.as_ref(),
        &grant,
        &admission,
    )
    .await
    .map_err(|error| Box::new(project_exception(&error)))?;
    reject_clientside_metadata_tags_check(stage.settings.reject_clientside_metadata_tags, &facts)?;
    enforce_user_param_check(stage.settings.enforce_user_param, &facts)
        .map_err(|error| Box::new(ordinary_exception(&error.to_string())))?;
    if let Some(denial) = global_proxy_budget_check(
        money(stage.settings.max_budget),
        rule.global_spend,
        skip,
        &stage.route,
    ) {
        return Err(Box::new(budget_exception(&denial)));
    }
    Ok(())
}

// B757's ordinary mapping: a bare project, user or tag failure is the 401 auth_error
fn project_exception(error: &ProjectError) -> ProxyException {
    match error {
        ProjectError::Blocked { .. } => ordinary_exception(&error.to_string()),
        ProjectError::Grant(grant) => grant_exception(grant),
        ProjectError::Budget(budget) => budget_failure_exception(budget),
    }
}

fn grant_exception(error: &EntityModelAccessError) -> ProxyException {
    match error {
        EntityModelAccessError::Denied(denial) => denial.as_proxy_exception().clone(),
        EntityModelAccessError::Member(budget) => team_budget_exception(budget),
        EntityModelAccessError::Project(lookup) => org_lookup_exception(lookup),
        EntityModelAccessError::RecurseDepth { .. } | EntityModelAccessError::TeamRow(_) => {
            ordinary_exception(&error.to_string())
        }
    }
}

fn budget_failure_exception(error: &TenantBudgetError) -> ProxyException {
    match error {
        TenantBudgetError::Denied(denial) => budget_exception(denial),
        TenantBudgetError::Spend(failure) => spend_failure_exception(failure),
        TenantBudgetError::Tag(_) => ordinary_exception(&error.to_string()),
    }
}

fn budget_exception(denial: &Denial) -> ProxyException {
    let failure = AuthFailure::BudgetExceeded {
        message: denial.message.clone(),
        status_code: Some(denial.status_code),
        llm_provider: String::new(),
    };
    as_proxy_exception(&failure).exception().clone()
}

fn ordinary_exception(message: &str) -> ProxyException {
    let failure = AuthFailure::Other {
        message: message.to_owned(),
        param: None,
    };
    as_proxy_exception(&failure).exception().clone()
}

const fn money(budget: MaxBudget) -> Money {
    match budget {
        MaxBudget::Finite(amount) => Money::Finite(amount),
        MaxBudget::Nan => Money::Nan,
        MaxBudget::PositiveInfinity => Money::PositiveInfinity,
        MaxBudget::NegativeInfinity => Money::NegativeInfinity,
    }
}

// a rule with no reader cannot verify project spend, which raises the 503
struct UnconfiguredReader;

#[async_trait]
impl SpendReader for UnconfiguredReader {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        Err(SpendReadError::Unverifiable {
            counter_key: query.counter_key.to_owned(),
        })
    }
}
