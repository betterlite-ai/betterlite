use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::database_reencryption::DEFAULT_RECURSE_DEPTH;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::BudgetExceededDetails;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::auth_model_budgets::checks::ModelMaxBudget;
use crate::auth_model_budgets::fallback::{
    KeyBudgetCall, KeyBudgetError, check_key_budget_fallback,
};
use crate::auth_model_budgets::limits::{
    KeyModelBudgetLimiter, TeamModelBudgetLimiter, UserModelBudgetLimiter,
};
use crate::auth_model_budgets::request_views::{ModelRewrite, RewriteFailure};
use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_model_patterns::pattern::UnresolvedProviderInference;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

pub(crate) fn token(fields: &[(&str, JsonData)]) -> UserAPIKeyAuth {
    let values: BTreeMap<String, JsonData> = fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect();
    UserAPIKeyAuth::construct(AuthInput::Dict(values)).expect("the key fixture constructs")
}

pub(crate) fn text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

pub(crate) fn names(values: &[&str]) -> JsonData {
    JsonData::Array(values.iter().map(|value| text(value)).collect())
}

pub(crate) fn budget_map(entries: &[(&str, i64)]) -> JsonData {
    JsonData::Object(
        entries
            .iter()
            .map(|(model, limit)| ((*model).to_owned(), JsonData::Number(Number::from(*limit))))
            .collect(),
    )
}

pub(crate) fn exceeded() -> BudgetExceededDetails {
    BudgetExceededDetails {
        current_cost: Decimal::new(125, 1),
        max_budget: Decimal::new(100, 1),
        message: Some("Budget has been exceeded!".to_owned()),
        provider: "openai".to_owned(),
        entity_type: Some("key".to_owned()),
        entity_id: Some("hashed-key".to_owned()),
        status_code: 422,
    }
}

pub(crate) fn budget_error() -> ErrorClass {
    ErrorClass::BudgetExceeded(Box::new(exceeded()))
}

pub(crate) fn other_error() -> ErrorClass {
    ErrorClass::Unclassified(Box::new(ErrorContext {
        message: "the counter store failed".to_owned(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }))
}

fn outcome(failure: &Mutex<Option<ErrorClass>>, within: bool) -> Result<bool, ErrorClass> {
    let pending = failure
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    if let Some(error) = pending {
        return Err(error);
    }
    Ok(within)
}

pub(crate) struct ScriptedKeyLimiter {
    pub(crate) failing: Mutex<Vec<String>>,
    pub(crate) failure: Mutex<Option<ErrorClass>>,
    pub(crate) within: bool,
    pub(crate) fallback: Mutex<Option<String>>,
    pub(crate) key_calls: Mutex<Vec<String>>,
    pub(crate) fallback_calls: Mutex<Vec<String>>,
}

impl Default for ScriptedKeyLimiter {
    fn default() -> Self {
        Self::new(&[], None)
    }
}

impl ScriptedKeyLimiter {
    // every field is named here, so this constructor also holds the default values
    pub(crate) fn new(failing: &[&str], fallback: Option<&str>) -> Self {
        Self {
            failing: Mutex::new(failing.iter().map(ToString::to_string).collect()),
            failure: Mutex::new(None),
            within: false,
            fallback: Mutex::new(fallback.map(str::to_owned)),
            key_calls: Mutex::new(Vec::new()),
            fallback_calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn with_fallback(mut self, fallback: &str) -> Self {
        self.fallback = Mutex::new(Some(fallback.to_owned()));
        self
    }

    fn record(calls: &Mutex<Vec<String>>, model: &str) {
        calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
    }
}

#[async_trait]
impl KeyModelBudgetLimiter for ScriptedKeyLimiter {
    async fn is_key_within_model_budget(
        &self,
        _token: &UserAPIKeyAuth,
        model: &str,
    ) -> Result<bool, ErrorClass> {
        Self::record(&self.key_calls, model);
        let failing = self
            .failing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|name| name == model);
        if failing {
            return Err(budget_error());
        }
        outcome(&self.failure, self.within)
    }

    async fn fallback_model_within_budget(
        &self,
        _token: &UserAPIKeyAuth,
        model: &str,
    ) -> Result<Option<String>, ErrorClass> {
        let fallback = self
            .fallback
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Self::record(&self.fallback_calls, model);
        Ok(fallback)
    }
}

#[derive(Default)]
pub(crate) struct ScriptedUserLimiter {
    pub(crate) calls: Mutex<Vec<(String, String)>>,
    pub(crate) seen: Mutex<Vec<(String, ModelMaxBudget)>>,
    pub(crate) failure: Mutex<Option<ErrorClass>>,
    pub(crate) within: bool,
}

impl ScriptedUserLimiter {
    fn record(&self, user_id: &str, budget: &ModelMaxBudget, model: &str) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((user_id.to_owned(), model.to_owned()));
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((user_id.to_owned(), budget.clone()));
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
        outcome(&self.failure, self.within)
    }
}

pub(crate) struct SeenTeam {
    pub(crate) team_id: String,
    pub(crate) model: String,
    pub(crate) budget: ModelMaxBudget,
    pub(crate) key_budget: Option<ModelMaxBudget>,
}

#[derive(Default)]
pub(crate) struct ScriptedTeamLimiter {
    pub(crate) seen: Mutex<Vec<SeenTeam>>,
    pub(crate) failure: Mutex<Option<ErrorClass>>,
    pub(crate) within: bool,
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
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SeenTeam {
                team_id: team_id.to_owned(),
                model: model.to_owned(),
                budget: team_model_max_budget.clone(),
                key_budget: key_model_max_budget.cloned(),
            });
        outcome(&self.failure, self.within)
    }
}

pub(crate) struct RecordingRewrite {
    pub(crate) calls: Mutex<Vec<&'static str>>,
    pub(crate) request_model: Mutex<Option<String>>,
    pub(crate) parsed_body: Mutex<Option<String>>,
    pub(crate) json_cache: Mutex<Option<String>>,
    pub(crate) serialized_body: Mutex<Option<String>>,
    pub(crate) path_model: Mutex<Option<String>>,
    pub(crate) path_exists: bool,
    pub(crate) fail_serialization: bool,
}

impl Default for RecordingRewrite {
    fn default() -> Self {
        Self::new(true)
    }
}

impl RecordingRewrite {
    pub(crate) fn new(path_exists: bool) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            request_model: Mutex::new(Some("gpt-4".to_owned())),
            parsed_body: Mutex::new(None),
            json_cache: Mutex::new(None),
            serialized_body: Mutex::new(None),
            path_model: Mutex::new(Some("gpt-4".to_owned())),
            path_exists,
            fail_serialization: false,
        }
    }

    pub(crate) fn models(&self) -> [Option<String>; 5] {
        [
            self.request_model.lock().unwrap().clone(),
            self.parsed_body.lock().unwrap().clone(),
            self.json_cache.lock().unwrap().clone(),
            self.serialized_body.lock().unwrap().clone(),
            self.path_model.lock().unwrap().clone(),
        ]
    }

    fn current(&self) -> String {
        self.request_model
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .unwrap_or_default()
    }

    fn record(&self, call: &'static str) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    pub(crate) fn calls(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn failing_serialization(mut self) -> Self {
        self.fail_serialization = true;
        self
    }
}

impl ModelRewrite for RecordingRewrite {
    fn set_request_model(&mut self, model: &str) {
        self.record("request_data");
        *self
            .request_model
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(model.to_owned());
    }

    fn refresh_parsed_body(&mut self) {
        self.record("parsed_body");
        let model = self.current();
        *self
            .parsed_body
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(model);
    }

    fn refresh_json_cache(&mut self) {
        self.record("json_cache");
        let model = self.current();
        *self
            .json_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(model);
    }

    fn refresh_serialized_body(&mut self) -> Result<(), RewriteFailure> {
        self.record("serialized_body");
        if self.fail_serialization {
            return Err(RewriteFailure);
        }
        let model = self.current();
        *self
            .serialized_body
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(model);
        Ok(())
    }

    fn set_path_model(&mut self, model: &str) {
        self.record("path_params");
        if self.path_exists {
            *self
                .path_model
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(model.to_owned());
        }
    }
}

pub(crate) struct Access {
    pub(crate) aliases: BTreeMap<String, String>,
    pub(crate) provider: UnresolvedProviderInference,
    pub(crate) depth: i64,
}

impl Default for Access {
    fn default() -> Self {
        Self::with_depth(DEFAULT_RECURSE_DEPTH)
    }
}

impl Access {
    // every field is named here, so this constructor also holds the default values
    pub(crate) fn with_depth(depth: i64) -> Self {
        Self {
            aliases: BTreeMap::new(),
            provider: UnresolvedProviderInference,
            depth,
        }
    }

    pub(crate) fn context(&self) -> ModelAccessContext<'_> {
        ModelAccessContext {
            view: None,
            global_model_aliases: &self.aliases,
            compaction: None,
            provider: &self.provider,
            max_recurse_depth: self.depth,
        }
    }
}

pub(crate) async fn key_check(
    limiter: &ScriptedKeyLimiter,
    key: &UserAPIKeyAuth,
    request: &mut RecordingRewrite,
    access: &Access,
    request_model: Option<&str>,
) -> Result<(), KeyBudgetError> {
    check_key_budget_fallback(
        KeyBudgetCall {
            limiter,
            token: key,
            model_name: "gpt-4",
            request_model,
        },
        request,
        &access.context(),
        None,
    )
    .await
}
