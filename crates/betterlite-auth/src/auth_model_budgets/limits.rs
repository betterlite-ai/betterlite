use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::error::ErrorClass;

use crate::resolved_key::auth::UserAPIKeyAuth;

// user_api_key_auth.py L306-L325: the limiter protocols this caller enforces, installed over B3006 by B8481
#[async_trait]
pub trait KeyModelBudgetLimiter: Send + Sync {
    async fn is_key_within_model_budget(
        &self,
        token: &UserAPIKeyAuth,
        model: &str,
    ) -> Result<bool, ErrorClass>;

    async fn fallback_model_within_budget(
        &self,
        token: &UserAPIKeyAuth,
        model: &str,
    ) -> Result<Option<String>, ErrorClass>;
}

// B8481 installs the limiter implementations behind these protocols; their counters stay in B3006
#[async_trait]
pub trait UserModelBudgetLimiter: Send + Sync {
    async fn is_user_within_model_budget(
        &self,
        user_id: &str,
        user_model_max_budget: &BTreeMap<String, JsonData>,
        model: &str,
    ) -> Result<bool, ErrorClass>;
}

#[async_trait]
pub trait TeamModelBudgetLimiter: Send + Sync {
    async fn is_team_within_model_budget(
        &self,
        team_id: &str,
        team_model_max_budget: &BTreeMap<String, JsonData>,
        key_model_max_budget: Option<&BTreeMap<String, JsonData>>,
        model: &str,
    ) -> Result<bool, ErrorClass>;
}
