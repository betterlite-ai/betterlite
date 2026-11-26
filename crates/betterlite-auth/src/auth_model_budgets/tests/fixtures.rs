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
use crate::auth_model_budgets::limits::{
    KeyModelBudgetLimiter, TeamModelBudgetLimiter, UserModelBudgetLimiter,
};
use crate::auth_model_budgets::request_views::{ModelRewrite, RewriteFailure};
use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_model_patterns::pattern::UnresolvedProviderInference;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

pub(crate) fn token(fields: &[(&str, JsonData)]) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect();
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the key fixture constructs")
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

#[derive(Default)]
pub(crate) struct ScriptedKeyLimiter {
    pub(crate) failing: Mutex<Vec<String>>,
    pub(crate) failure: Mutex<Option<ErrorClass>>,
    pub(crate) within: bool,
    pub(crate) fallback: Mutex<Option<String>>,
    pub(crate) key_calls: Mutex<Vec<String>>,
    pub(crate) fallback_calls: Mutex<Vec<String>>,
}

#[async_trait]
impl KeyModelBudgetLimiter for ScriptedKeyLimiter {
    async fn is_key_within_model_budget(
        &self,
        _token: &UserAPIKeyAuth,
        model: &str,
    ) -> Result<bool, ErrorClass> {
        self.key_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        if let Some(error) = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            return Err(error);
        }
        let failing = self
            .failing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|name| name == model);
        if failing {
            return Err(budget_error());
        }
        Ok(self.within)
    }

    async fn fallback_model_within_budget(
        &self,
        _token: &UserAPIKeyAuth,
        model: &str,
    ) -> Result<Option<String>, ErrorClass> {
        self.fallback_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        Ok(self
            .fallback
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }
}

#[derive(Default)]
pub(crate) struct ScriptedUserLimiter {
    pub(crate) calls: Mutex<Vec<String>>,
    pub(crate) seen: Mutex<Vec<(String, ModelMaxBudget)>>,
    pub(crate) failure: Mutex<Option<ErrorClass>>,
    pub(crate) within: bool,
}

#[async_trait]
impl UserModelBudgetLimiter for ScriptedUserLimiter {
    async fn is_user_within_model_budget(
        &self,
        user_id: &str,
        user_model_max_budget: &ModelMaxBudget,
        model: &str,
    ) -> Result<bool, ErrorClass> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{user_id}:{model}"));
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((user_id.to_owned(), user_model_max_budget.clone()));
        if let Some(error) = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            return Err(error);
        }
        Ok(self.within)
    }
}

#[derive(Default)]
pub(crate) struct ScriptedTeamLimiter {
    pub(crate) calls: Mutex<Vec<String>>,
    pub(crate) key_budgets: Mutex<Vec<Option<ModelMaxBudget>>>,
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
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{team_id}:{model}:{}", team_model_max_budget.len()));
        self.key_budgets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key_model_max_budget.cloned());
        if let Some(error) = self
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            return Err(error);
        }
        Ok(self.within)
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
        Self {
            calls: Mutex::new(Vec::new()),
            request_model: Mutex::new(Some("gpt-4".to_owned())),
            parsed_body: Mutex::new(None),
            json_cache: Mutex::new(None),
            serialized_body: Mutex::new(None),
            path_model: Mutex::new(Some("gpt-4".to_owned())),
            path_exists: true,
            fail_serialization: false,
        }
    }
}

impl RecordingRewrite {
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
        Self {
            aliases: BTreeMap::new(),
            provider: UnresolvedProviderInference,
            depth: DEFAULT_RECURSE_DEPTH,
        }
    }
}

impl Access {
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
