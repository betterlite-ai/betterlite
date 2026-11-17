use std::collections::BTreeSet;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_server::key_fields::values::Amount;
use betterlite_server::proxy_request_contracts::customer::{
    AllowedModelRegion, NewCustomerRequest,
};
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
struct CustomerView<'a> {
    user_id: &'a String,
    alias: Option<&'a String>,
    blocked: bool,
    budget_id: Option<&'a String>,
    spend: Option<&'a Amount>,
    allowed_model_region: Option<AllowedModelRegion>,
    default_model: Option<&'a String>,
    max_budget: Option<&'a Amount>,
    soft_budget: Option<&'a Amount>,
    max_parallel_requests: Option<i64>,
    tpm_limit: Option<i64>,
    rpm_limit: Option<i64>,
    tpd_limit: Option<i64>,
    budget_duration: Option<&'a String>,
    supplied: &'a BTreeSet<String>,
}

pub(super) fn answer(body: &Value) -> Result<Value, Box<HelperError>> {
    let request = NewCustomerRequest::from_value(body).or_value_error()?;
    let view = CustomerView {
        user_id: &request.user_id,
        alias: request.alias.as_ref(),
        blocked: request.blocked,
        budget_id: request.budget_id.as_ref(),
        spend: request.spend.as_ref(),
        allowed_model_region: request.allowed_model_region,
        default_model: request.default_model.as_ref(),
        max_budget: request.budget.max_budget.as_ref(),
        soft_budget: request.budget.soft_budget.as_ref(),
        max_parallel_requests: request.budget.max_parallel_requests,
        tpm_limit: request.budget.tpm_limit,
        rpm_limit: request.budget.rpm_limit,
        tpd_limit: request.budget.tpd_limit,
        budget_duration: request.budget.budget_duration.as_ref(),
        supplied: &request.budget.supplied,
    };
    serde_json::to_value(view).or_value_error()
}
