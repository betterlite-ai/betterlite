use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(super) enum Call {
    ValidateFiniteSpend(SpendCall),
    ValidateBudgetDuration(DurationCall),
    ValidateTeamModelMaxBudget(TeamBudgetCall),
    CallerUserIdForNonAdmin(CallerCall),
    CheckPassthroughRoutes(PassthroughCall),
    CheckGuardrailOptOut(GuardrailCall),
    SetObjectMetadataField(SetMetadataCall),
    UpdateMetadataField(UpdateFieldCall),
    UpdateMetadataFields(UpdateFieldsCall),
}

#[derive(Debug, Deserialize)]
pub(super) struct SpendCall {
    #[serde(default)]
    pub(super) spend: Option<JsonData>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DurationCall {
    #[serde(default)]
    pub(super) budget_duration: Option<String>,
    #[serde(default)]
    pub(super) status_code: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TeamBudgetCall {
    #[serde(default)]
    pub(super) model_max_budget: Option<BTreeMap<String, BudgetInput>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct BudgetInput {
    #[serde(default)]
    pub(super) max_budget: Option<JsonData>,
    #[serde(default)]
    pub(super) budget_duration: Option<String>,
    #[serde(default)]
    pub(super) tpm_limit: Option<i64>,
    #[serde(default)]
    pub(super) rpm_limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CallerCall {
    #[serde(default)]
    pub(super) user_id: Option<String>,
    #[serde(default)]
    pub(super) user_role: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PassthroughCall {
    #[serde(default)]
    pub(super) user_role: Option<String>,
    #[serde(default)]
    pub(super) allowed_passthrough_routes: Option<Vec<JsonData>>,
    #[serde(default)]
    pub(super) metadata: Option<JsonData>,
    #[serde(default = "default_entity")]
    pub(super) entity: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct GuardrailCall {
    #[serde(default)]
    pub(super) user_role: Option<String>,
    #[serde(default)]
    pub(super) disable_global_guardrails: Option<bool>,
    #[serde(default)]
    pub(super) metadata: Option<JsonData>,
    #[serde(default)]
    pub(super) existing_metadata: Option<JsonData>,
    #[serde(default = "default_entity")]
    pub(super) entity: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetMetadataCall {
    #[serde(default)]
    pub(super) metadata: Option<BTreeMap<String, JsonData>>,
    pub(super) field_name: String,
    pub(super) value: JsonData,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateFieldCall {
    pub(super) updated_kv: BTreeMap<String, JsonData>,
    pub(super) field_name: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateFieldsCall {
    pub(super) updated_kv: BTreeMap<String, JsonData>,
}

fn default_entity() -> String {
    "key".to_owned()
}
