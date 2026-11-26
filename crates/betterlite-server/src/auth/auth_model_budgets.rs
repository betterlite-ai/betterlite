use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::{Body, Bytes};
use axum::http::Request;
use betterlite_auth::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use betterlite_auth::auth_model_budgets::checks::{
    check_team_model_budget, check_user_model_budget,
};
use betterlite_auth::auth_model_budgets::fallback::{
    KeyBudgetCall, KeyBudgetError, check_key_budget_fallback,
};
use betterlite_auth::auth_model_budgets::limits::{
    KeyModelBudgetLimiter, TeamModelBudgetLimiter, UserModelBudgetLimiter,
};
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::UnresolvedProviderInference;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::auth_model_budgets::Settings;
use betterlite_config::sections::database_reencryption::DEFAULT_RECURSE_DEPTH;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::BudgetExceededDetails;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::model_registration::JsonObject;

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;
use body_views::BodyViews;

mod body_views;
mod tests;

// What the authentication service resolved before this stage: the caller's token, the models the checks cover and the limiter protocols B8481 implements.
pub struct ModelBudgetRule {
    pub auth: Option<UserAPIKeyAuth>,
    pub models: Vec<String>,
    pub model_name: Option<String>,
    pub request_model: Option<String>,
    pub user_limiter: Option<Arc<dyn UserModelBudgetLimiter>>,
    pub team_limiter: Option<Arc<dyn TeamModelBudgetLimiter>>,
    pub key_limiter: Option<Arc<dyn KeyModelBudgetLimiter>>,
    pub global_model_aliases: BTreeMap<String, String>,
    pub max_recurse_depth: i64,
    pub path_params: Option<JsonObject>,
    applied: Mutex<Option<Bytes>>,
}

impl Default for ModelBudgetRule {
    fn default() -> Self {
        Self {
            auth: None,
            models: Vec::new(),
            model_name: None,
            request_model: None,
            user_limiter: None,
            team_limiter: None,
            key_limiter: None,
            global_model_aliases: BTreeMap::new(),
            max_recurse_depth: DEFAULT_RECURSE_DEPTH,
            path_params: None,
            applied: Mutex::new(None),
        }
    }
}

impl fmt::Debug for ModelBudgetRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelBudgetRule")
            .field("has_auth", &self.auth.is_some())
            .field("models", &self.models)
            .field("model_name", &self.model_name)
            .field("request_model", &self.request_model)
            .field("has_user_limiter", &self.user_limiter.is_some())
            .field("has_team_limiter", &self.team_limiter.is_some())
            .field("has_key_limiter", &self.key_limiter.is_some())
            .field("max_recurse_depth", &self.max_recurse_depth)
            .finish_non_exhaustive()
    }
}

impl ModelBudgetRule {
    // the rewritten request body, which the auth middleware forwards in place of the buffered one
    #[must_use]
    pub fn applied_body(&self) -> Option<Bytes> {
        self.applied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Debug)]
pub struct ModelBudgetStageInput {
    pub context: AuthContext,
    pub rule: Arc<ModelBudgetRule>,
    pub settings: Arc<Settings>,
    pub body: Option<Bytes>,
}

// the authentication service inserts the rule once it resolved the caller's key; without one the stage is inert
pub(crate) fn rule_for_request(request: &Request<Body>) -> Arc<ModelBudgetRule> {
    request
        .extensions()
        .get::<Arc<ModelBudgetRule>>()
        .map_or_else(|| Arc::new(ModelBudgetRule::default()), Arc::clone)
}

// a router mounted without the proxy state has no loaded section, so the pinned defaults apply
fn settings_for_request(request: &Request<Body>) -> Arc<Settings> {
    request.extensions().get::<Arc<ServerState>>().map_or_else(
        || Arc::new(Settings::default()),
        |state| Arc::clone(&state.auth_model_budgets.settings),
    )
}

pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
    body: Option<&Bytes>,
) -> Result<ModelBudgetStageInput, AuthenticationFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    Ok(ModelBudgetStageInput {
        context,
        rule: rule_for_request(request),
        settings: settings_for_request(request),
        body: body.cloned(),
    })
}

// the auth stage loop holds the input across the await, so the prefetched stage comes back wrapped
pub(crate) async fn prefetched(
    stage: Box<ModelBudgetStageInput>,
) -> Result<StageInput, StageFailure> {
    prefetch(&stage).await.map_err(StageFailure::ModelBudgets)?;
    Ok(StageInput::AuthModelBudgets(stage))
}

// The checks run in LiteLLM's order: the internal user's cap, the team's cap, then the key's cap with its fallback.
pub(crate) async fn prefetch(stage: &ModelBudgetStageInput) -> Result<(), Box<ProxyException>> {
    let rule = &stage.rule;
    let Some(auth) = rule.auth.as_ref() else {
        return Ok(());
    };
    if let Some(limiter) = rule.user_limiter.as_deref() {
        check_user_model_budget(limiter, auth, &rule.models)
            .await
            .map_err(|error| Box::new(failure_exception(&error)))?;
    }
    if let Some(limiter) = rule.team_limiter.as_deref() {
        check_team_model_budget(limiter, auth, &rule.models)
            .await
            .map_err(|error| Box::new(failure_exception(&error)))?;
    }
    let Some(limiter) = rule.key_limiter.as_deref() else {
        return Ok(());
    };
    let applied = check_key_budget(rule, auth, limiter, stage.body.as_ref()).await?;
    *rule.applied.lock().unwrap_or_else(PoisonError::into_inner) = applied;
    Ok(())
}

// a budget error or a denied fallback leaves the body as it was, so no rewrite is recorded
async fn check_key_budget(
    rule: &ModelBudgetRule,
    auth: &UserAPIKeyAuth,
    limiter: &dyn KeyModelBudgetLimiter,
    body: Option<&Bytes>,
) -> Result<Option<Bytes>, Box<ProxyException>> {
    let Some(model_name) = rule.model_name.as_deref() else {
        return Ok(None);
    };
    let provider = UnresolvedProviderInference;
    let context = ModelAccessContext {
        view: None,
        global_model_aliases: &rule.global_model_aliases,
        compaction: None,
        provider: &provider,
        max_recurse_depth: rule.max_recurse_depth,
    };
    let mut views = BodyViews::new(body, rule.path_params.clone());
    let call = KeyBudgetCall {
        limiter,
        token: auth,
        model_name,
        request_model: rule.request_model.as_deref(),
    };
    check_key_budget_fallback(call, &mut views, &context, None)
        .await
        .map_err(|error| Box::new(key_budget_exception(&error)))?;
    Ok(views.serialized_body.map(Bytes::from))
}

fn key_budget_exception(error: &KeyBudgetError) -> ProxyException {
    match error {
        KeyBudgetError::Budget(details) => budget_exception(details),
        KeyBudgetError::Limiter(class) => failure_exception(class),
        KeyBudgetError::Access(access) => access.denial().map_or_else(
            || other_exception(&access.to_string()),
            |denial| denial.as_proxy_exception(),
        ),
        KeyBudgetError::Rewrite(_) => other_exception(&error.to_string()),
    }
}

fn failure_exception(error: &ErrorClass) -> ProxyException {
    match error {
        ErrorClass::BudgetExceeded(details) => budget_exception(details),
        other => other_exception(&other.to_string()),
    }
}

// a budget rejection is budget_exceeded with the rejection's own status and a null param
fn budget_exception(details: &BudgetExceededDetails) -> ProxyException {
    let failure = AuthFailure::BudgetExceeded {
        message: details
            .message
            .clone()
            .unwrap_or_else(|| default_message(details)),
        status_code: Some(details.status_code),
        llm_provider: details.provider.clone(),
    };
    as_proxy_exception(&failure).exception().clone()
}

fn other_exception(message: &str) -> ProxyException {
    let failure = AuthFailure::Other {
        message: message.to_owned(),
        param: None,
    };
    as_proxy_exception(&failure).exception().clone()
}

fn default_message(details: &BudgetExceededDetails) -> String {
    format!(
        "Budget has been exceeded! Current cost: {}, Max budget: {}",
        details.current_cost, details.max_budget
    )
}
