use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::{Body, Bytes};
use axum::http::Request;
use betterlite_auth::auth_errors::conversion::{budget_details_exception, other_message_exception};
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
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::model_registration::JsonObject;

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;
use body_views::BodyViews;

pub(crate) mod body_views;
mod tests;

// the middleware and this stage share one handle so a fallback's body snapshots reach the downstream handler
#[derive(Clone, Debug, Default)]
pub struct SharedViews(pub(crate) Arc<Mutex<Option<BodyViews>>>);

// What the authentication service resolved before this stage: the caller's token, the models the checks cover and the limiter protocols B8481 implements.
#[derive(Clone)]
pub struct ModelBudgetRule {
    pub auth: Option<UserAPIKeyAuth>,
    pub models: Vec<String>,
    pub model_name: Option<String>,
    pub user_limiter: Option<Arc<dyn UserModelBudgetLimiter>>,
    pub team_limiter: Option<Arc<dyn TeamModelBudgetLimiter>>,
    pub key_limiter: Option<Arc<dyn KeyModelBudgetLimiter>>,
    pub global_model_aliases: BTreeMap<String, String>,
    pub max_recurse_depth: i64,
    pub path_params: Option<JsonObject>,
}

impl Default for ModelBudgetRule {
    fn default() -> Self {
        Self {
            auth: None,
            models: Vec::new(),
            model_name: None,
            user_limiter: None,
            team_limiter: None,
            key_limiter: None,
            global_model_aliases: BTreeMap::new(),
            max_recurse_depth: DEFAULT_RECURSE_DEPTH,
            path_params: None,
        }
    }
}

impl fmt::Debug for ModelBudgetRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelBudgetRule")
            .field("has_auth", &self.auth.is_some())
            .field("models", &self.models)
            .field("model_name", &self.model_name)
            .field("has_user_limiter", &self.user_limiter.is_some())
            .field("has_team_limiter", &self.team_limiter.is_some())
            .field("has_key_limiter", &self.key_limiter.is_some())
            .field("max_recurse_depth", &self.max_recurse_depth)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct ModelBudgetStageInput {
    pub context: AuthContext,
    pub rule: Arc<ModelBudgetRule>,
    pub settings: Arc<Settings>,
    pub body: Option<Bytes>,
    pub views: SharedViews,
}

// the authentication service inserts the rule once it resolved the caller's key; without one the stage is inert
pub(crate) fn rule_for_request(request: &Request<Body>) -> Arc<ModelBudgetRule> {
    let mut rule = request
        .extensions()
        .get::<Arc<ModelBudgetRule>>()
        .map_or_else(ModelBudgetRule::default, |rule| rule.as_ref().clone());
    if let Some(state) = request.extensions().get::<Arc<ServerState>>() {
        let slot = &state.auth_model_budgets;
        rule.key_limiter = rule.key_limiter.or_else(|| slot.key_limiter.clone());
        rule.user_limiter = rule.user_limiter.or_else(|| slot.user_limiter.clone());
        rule.team_limiter = rule.team_limiter.or_else(|| slot.team_limiter.clone());
    }
    Arc::new(rule)
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
        views: request
            .extensions()
            .get::<SharedViews>()
            .cloned()
            .unwrap_or_default(),
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
    check_key_budget(stage, auth, limiter).await
}

// a budget error or a denied fallback leaves the body as it was, so no rewrite is recorded
async fn check_key_budget(
    stage: &ModelBudgetStageInput,
    auth: &UserAPIKeyAuth,
    limiter: &dyn KeyModelBudgetLimiter,
) -> Result<(), Box<ProxyException>> {
    let rule = &stage.rule;
    let Some(model_name) = rule.model_name.as_deref() else {
        return Ok(());
    };
    let provider = UnresolvedProviderInference;
    let context = ModelAccessContext {
        view: None,
        global_model_aliases: &rule.global_model_aliases,
        compaction: None,
        provider: &provider,
        max_recurse_depth: rule.max_recurse_depth,
    };
    let mut views = BodyViews::new(stage.body.as_ref(), rule.path_params.clone());
    let request_model = views
        .request_data
        .as_ref()
        .and_then(|data| data.get("model"))
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    let call = KeyBudgetCall {
        limiter,
        token: auth,
        model_name,
        request_model: request_model.as_deref(),
    };
    let result = check_key_budget_fallback(call, &mut views, &context, None).await;
    *stage.views.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(views);
    result.map_err(|error| Box::new(key_budget_exception(&error)))
}

fn key_budget_exception(error: &KeyBudgetError) -> ProxyException {
    match error {
        KeyBudgetError::Budget(details) => budget_details_exception(details),
        KeyBudgetError::Limiter(class) => failure_exception(class),
        KeyBudgetError::Access(access) => access.denial().map_or_else(
            || other_message_exception(&access.to_string()),
            |denial| denial.as_proxy_exception().clone(),
        ),
        KeyBudgetError::Rewrite(_) => other_message_exception(&error.to_string()),
    }
}

fn failure_exception(error: &ErrorClass) -> ProxyException {
    if let ErrorClass::BudgetExceeded(details) = error {
        return budget_details_exception(details);
    }
    other_message_exception(&class_message(error))
}
