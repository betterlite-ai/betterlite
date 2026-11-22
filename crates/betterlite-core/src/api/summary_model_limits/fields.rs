use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use betterlite_db::budget_permission_rows::JsonData;
use secrecy::SecretString;
use serde::Deserialize;

pub type BudgetMap = BTreeMap<String, JsonData>;

// the UserAPIKeyAuth fields the summary budget and rate checks read, taken from the principal's serde form
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct LimitFields {
    pub token: Option<SecretString>,
    pub key_alias: Option<String>,
    pub model_max_budget: Option<BudgetMap>,
    pub user_id: Option<String>,
    pub user_model_max_budget: Option<BudgetMap>,
    pub team_id: Option<String>,
    pub team_model_max_budget: Option<BudgetMap>,
    pub end_user_id: Option<String>,
    pub end_user_model_max_budget: Option<BudgetMap>,
    pub metadata: Option<BTreeMap<String, JsonData>>,
    pub parent_otel_span: Option<ParentOtelSpan>,
}
