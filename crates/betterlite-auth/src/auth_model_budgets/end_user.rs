use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::identity_rows::Payload;
use betterlite_db::identity_rows::end_user::Region;
use rust_decimal::Decimal;

use crate::resolved_key::auth::UserAPIKeyAuth;

use super::checks::ModelMaxBudget;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EndUserParams {
    pub end_user_id: Option<String>,
    pub end_user_tpm_limit: Option<i64>,
    pub end_user_rpm_limit: Option<i64>,
    pub end_user_tpd_limit: Option<i64>,
    pub end_user_max_budget: Option<Decimal>,
    pub end_user_model_max_budget: Option<ModelMaxBudget>,
    pub allowed_model_region: Option<Region>,
}

// _apply_budget_limits_to_end_user_params: a nonnull row field wins even when it is zero or an empty map
pub fn apply_budget_limits_to_end_user_params(
    params: &mut EndUserParams,
    budget: &LiteLLMBudgetTable,
    end_user_id: Option<&str>,
) {
    if let Some(tpm_limit) = budget.tpm_limit {
        params.end_user_tpm_limit = Some(tpm_limit);
    }
    if let Some(rpm_limit) = budget.rpm_limit {
        params.end_user_rpm_limit = Some(rpm_limit);
    }
    if let Some(tpd_limit) = budget.tpd_limit {
        params.end_user_tpd_limit = Some(tpd_limit);
    }
    if let Some(max_budget) = budget.max_budget {
        params.end_user_max_budget = Some(max_budget);
    }
    if let Some(model_max_budget) = budget.model_max_budget.as_ref() {
        params.end_user_model_max_budget = Some(model_max_budget.clone());
    }
    tracing::debug!(end_user = ?end_user_id, "Applied budget limits to end user");
}

// update_valid_token_with_end_user_params: the id is always assigned, the other fields only when the row had them
pub fn update_valid_token_with_end_user_params(token: &mut UserAPIKeyAuth, params: &EndUserParams) {
    token.view.end_user_id.clone_from(&params.end_user_id);
    if let Some(tpm_limit) = params.end_user_tpm_limit {
        token.view.end_user_tpm_limit = Some(tpm_limit);
    }
    if let Some(rpm_limit) = params.end_user_rpm_limit {
        token.view.end_user_rpm_limit = Some(rpm_limit);
    }
    if let Some(tpd_limit) = params.end_user_tpd_limit {
        token.view.end_user_tpd_limit = Some(tpd_limit);
    }
    if let Some(region) = params.allowed_model_region.clone() {
        token.extra.allowed_model_region = Some(region);
    }
    if let Some(model_max_budget) = params.end_user_model_max_budget.as_ref() {
        token.view.end_user_model_max_budget = Some(Payload(model_max_budget.clone()));
    }
}
