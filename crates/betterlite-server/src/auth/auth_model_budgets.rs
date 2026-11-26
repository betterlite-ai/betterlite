use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::{Body, Bytes};
use axum::http::Request;
use betterlite_auth::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use betterlite_auth::auth_model_budgets::checks::{
    check_team_model_budget, check_user_model_budget,
};
use betterlite_auth::auth_model_budgets::fallback::{KeyBudgetError, check_key_budget_fallback};
use betterlite_auth::auth_model_budgets::limits::{
    KeyModelBudgetLimiter, TeamModelBudgetLimiter, UserModelBudgetLimiter,
};
use betterlite_auth::auth_model_budgets::request_views::{ModelRewrite, RewriteFailure};
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::UnresolvedProviderInference;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::auth_model_budgets::Settings;
use betterlite_config::sections::database_reencryption::DEFAULT_RECURSE_DEPTH;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::BudgetExceededDetails;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::auth::auth_identity::AuthContext;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

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
    let mut views = BodyViews::new(stage.body.as_deref(), rule.path_params.clone());
    check_key_budget_fallback(
        limiter,
        auth,
        model_name,
        rule.request_model.as_deref(),
        &mut views,
        &context,
        None,
    )
    .await
    .map_err(|error| Box::new(key_budget_exception(&error)))?;
    let applied = views.serialized_body.map(Bytes::from);
    *rule.applied.lock().unwrap_or_else(PoisonError::into_inner) = applied;
    Ok(())
}

// The bodies LiteLLM keeps apart are one parsed object here, so a view the request did not carry stays absent.
struct BodyViews {
    request_data: Option<JsonObject>,
    parsed_body: Option<JsonObject>,
    json_cache: Option<JsonObject>,
    serialized_body: Option<Vec<u8>>,
    path_params: Option<JsonObject>,
}

impl BodyViews {
    fn new(body: Option<&Bytes>, path_params: Option<JsonObject>) -> Self {
        Self {
            request_data: body.and_then(|bytes| serde_json::from_slice::<JsonObject>(bytes).ok()),
            parsed_body: None,
            json_cache: None,
            serialized_body: None,
            path_params,
        }
    }

    fn mirror(&mut self, target: fn(&mut Self) -> Option<&mut JsonObject>) {
        let Some(data) = self.request_data.clone() else {
            return;
        };
        if let Some(view) = target(self) {
            *view = data;
        }
    }
}

impl ModelRewrite for BodyViews {
    fn set_request_model(&mut self, model: &str) {
        if let Some(data) = self.request_data.as_mut() {
            data.insert("model".to_owned(), Json::Text(model.to_owned()));
        }
    }

    fn refresh_parsed_body(&mut self) {
        self.mirror(|views| views.parsed_body.as_mut());
    }

    fn refresh_json_cache(&mut self) {
        self.mirror(|views| views.json_cache.as_mut());
    }

    fn refresh_serialized_body(&mut self) -> Result<(), RewriteFailure> {
        let Some(data) = self.request_data.as_ref() else {
            return Err(RewriteFailure);
        };
        let body = serde_json::to_vec(data).map_err(|_error| RewriteFailure)?;
        self.serialized_body = Some(body);
        Ok(())
    }

    fn set_path_model(&mut self, model: &str) {
        if let Some(params) = self.path_params.as_mut()
            && params.contains_key("model")
        {
            params.insert("model".to_owned(), Json::Text(model.to_owned()));
        }
    }
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
