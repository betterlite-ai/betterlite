use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use super::support::Failure;
use async_trait::async_trait;
use betterlite_auth::auth_model_budgets::checks::ModelMaxBudget;
use betterlite_auth::auth_model_budgets::global_spend::{
    ProxyBudgetAlert, ProxyBudgetAlertSink, ProxyBudgetRows,
};
use betterlite_auth::auth_model_budgets::limits::{
    KeyModelBudgetLimiter, TeamModelBudgetLimiter, UserModelBudgetLimiter,
};
use betterlite_auth::auth_model_budgets::request_views::{ModelRewrite, RewriteFailure};
use betterlite_auth::auth_model_budgets::rows::SpendReadError;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_cache::backend::{BackendError, CacheBackend, Ttl};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::BudgetExceededDetails;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::time::timeout;

// the BudgetExceededError a limiter raises for a model over its cap
pub(super) fn budget_exceeded(entity_type: &str, entity_id: &str) -> ErrorClass {
    ErrorClass::BudgetExceeded(Box::new(BudgetExceededDetails {
        current_cost: Decimal::new(125, 1),
        max_budget: Decimal::new(100, 1),
        message: Some("Budget has been exceeded!".to_owned()),
        provider: "openai".to_owned(),
        entity_type: Some(entity_type.to_owned()),
        entity_id: Some(entity_id.to_owned()),
        status_code: 422,
    }))
}

pub(super) struct ScriptedKeyLimiter {
    pub(super) failing: bool,
    pub(super) fallback: Option<String>,
    pub(super) calls: Mutex<Vec<String>>,
}

impl ScriptedKeyLimiter {
    fn record(&self, entry: String) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl KeyModelBudgetLimiter for ScriptedKeyLimiter {
    async fn is_key_within_model_budget(
        &self,
        _token: &UserAPIKeyAuth,
        model: &str,
    ) -> Result<bool, ErrorClass> {
        self.record(format!("key:{model}"));
        if self.failing {
            return Err(budget_exceeded("key", "hashed-key"));
        }
        Ok(true)
    }

    async fn fallback_model_within_budget(
        &self,
        _token: &UserAPIKeyAuth,
        model: &str,
    ) -> Result<Option<String>, ErrorClass> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("fallback:{model}"));
        Ok(self.fallback.clone())
    }
}

#[derive(Clone)]
pub(super) struct SeenUser {
    pub(super) user_id: String,
    pub(super) model: String,
    pub(super) budget: ModelMaxBudget,
}

// a model over its cap with the entity the BudgetExceededError names
fn refusal(
    failing_models: &[String],
    model: &str,
    entity_type: &str,
    entity_id: &str,
) -> Result<bool, ErrorClass> {
    if failing_models.iter().any(|name| name == model) {
        return Err(budget_exceeded(entity_type, entity_id));
    }
    Ok(true)
}

pub(super) struct ScriptedUserLimiter {
    pub(super) failing_models: Vec<String>,
    pub(super) seen: Mutex<Vec<SeenUser>>,
}

impl ScriptedUserLimiter {
    fn record(&self, user_id: &str, budget: &ModelMaxBudget, model: &str) {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SeenUser {
                user_id: user_id.to_owned(),
                model: model.to_owned(),
                budget: budget.clone(),
            });
    }

    pub(super) fn seen(&self) -> Vec<SeenUser> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl UserModelBudgetLimiter for ScriptedUserLimiter {
    async fn is_user_within_model_budget(
        &self,
        user_id: &str,
        user_model_max_budget: &ModelMaxBudget,
        model: &str,
    ) -> Result<bool, ErrorClass> {
        self.record(user_id, user_model_max_budget, model);
        refusal(&self.failing_models, model, "user", user_id)
    }
}

pub(super) struct ScriptedTeamLimiter {
    pub(super) failing_models: Vec<String>,
    pub(super) seen: Mutex<Vec<Value>>,
}

impl ScriptedTeamLimiter {
    pub(super) fn seen(&self) -> Vec<Value> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl TeamModelBudgetLimiter for ScriptedTeamLimiter {
    async fn is_team_within_model_budget(
        &self,
        team_id: &str,
        team_model_max_budget: &ModelMaxBudget,
        key_model_max_budget: Option<&ModelMaxBudget>,
        model: &str,
    ) -> Result<bool, ErrorClass> {
        let record = json!({
            "team_id": team_id,
            "model": model,
            "team_budget": JsonData::Object(team_model_max_budget.clone()),
            "key_budget": key_model_max_budget.map(|budget| JsonData::Object(budget.clone())),
        });
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
        refusal(&self.failing_models, model, "team", team_id)
    }
}

// the Starlette views _check_key_model_budget_with_fallback rewrites, kept apart as the request keeps them
pub(super) struct CaseRewrite {
    pub(super) request_data: Option<BTreeMap<String, JsonData>>,
    pub(super) parsed_body: Option<BTreeMap<String, JsonData>>,
    pub(super) json_cache: Option<BTreeMap<String, JsonData>>,
    pub(super) serialized_body: Option<Vec<u8>>,
    pub(super) path_params: Option<BTreeMap<String, JsonData>>,
}

impl ModelRewrite for CaseRewrite {
    fn set_request_model(&mut self, model: &str) {
        if let Some(data) = self.request_data.as_mut() {
            data.insert("model".to_owned(), JsonData::String(model.to_owned()));
        }
    }

    fn refresh_parsed_body(&mut self) {
        self.parsed_body.clone_from(&self.request_data);
    }

    fn refresh_json_cache(&mut self) {
        self.json_cache.clone_from(&self.request_data);
    }

    fn refresh_serialized_body(&mut self) -> Result<(), RewriteFailure> {
        let data = self.request_data.as_ref().ok_or(RewriteFailure)?;
        let body = serde_json::to_vec(data).map_err(|_error| RewriteFailure)?;
        self.serialized_body = Some(body);
        Ok(())
    }

    fn set_path_model(&mut self, model: &str) {
        if let Some(params) = self.path_params.as_mut()
            && params.contains_key("model")
        {
            params.insert("model".to_owned(), JsonData::String(model.to_owned()));
        }
    }
}

pub(super) struct SpendRows {
    pub(super) spend: Option<Decimal>,
    pub(super) gated: bool,
    pub(super) started: Semaphore,
    pub(super) release: Semaphore,
    pub(super) calls: AtomicUsize,
}

impl SpendRows {
    pub(super) async fn wait_until_started(&self) -> Result<(), Failure> {
        let _permit = timeout(Duration::from_secs(1), self.started.acquire())
            .await
            .map_err(|_error| Failure::Coordinator)?
            .map_err(|_error| Failure::Coordinator)?;
        Ok(())
    }
}

#[async_trait]
impl ProxyBudgetRows for SpendRows {
    async fn find_proxy_budget(&self) -> Result<Option<Decimal>, SpendReadError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.add_permits(1);
        if self.gated {
            let _release = self.release.acquire().await;
        }
        Ok(self.spend)
    }
}

#[derive(Default)]
pub(super) struct SpendCache {
    stored: Mutex<Option<CacheValue>>,
}

#[async_trait]
impl CacheBackend for SpendCache {
    fn default_ttl(&self) -> i64 {
        60
    }
    async fn async_get_cache(&self, _key: &str) -> Result<Option<CacheValue>, BackendError> {
        Ok(self
            .stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    async fn async_set_cache(
        &self,
        _key: &str,
        value: CacheValue,
        _ttl: Ttl,
    ) -> Result<(), BackendError> {
        *self.stored.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct AlertRecorder {
    pub(super) alerts: Mutex<Vec<ProxyBudgetAlert>>,
}

impl ProxyBudgetAlertSink for AlertRecorder {
    fn schedule(&self, alert: ProxyBudgetAlert) {
        self.alerts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(alert);
    }
}
