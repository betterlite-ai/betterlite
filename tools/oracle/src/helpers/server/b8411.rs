use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::key_fields::values::Amount;
use betterlite_server::proxy_request_contracts::coordination_redis::CoordinationRedisParams;
use betterlite_server::proxy_request_contracts::cost_estimate::CostEstimateRequest;
use betterlite_server::proxy_request_contracts::customer::{
    AllowedModelRegion, NewCustomerRequest,
};
use betterlite_server::proxy_request_contracts::key::{KeyRequest, ResetSpendRequest};
use betterlite_server::proxy_request_contracts::user::{
    NewUserRequest, NewUserTeams, UpdateUserRequest,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Key,
    ResetSpend,
    NewUser,
    UpdateUser,
    Customer,
    CostEstimate,
    RedisParams,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    body: JsonData,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Serialize)]
struct Outcome {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct NewUserView<'a> {
    user_email: Option<&'a String>,
    user_alias: Option<&'a String>,
    user_role: Option<UserRole>,
    max_budget: Option<&'a Amount>,
    teams: Option<&'a NewUserTeams>,
    auto_create_key: bool,
    send_invite_email: Option<bool>,
    sso_user_id: Option<&'a String>,
    organizations: Option<&'a Vec<String>>,
}

#[derive(Serialize)]
struct UpdateUserView<'a> {
    user_id: Option<&'a String>,
    user_email: Option<&'a String>,
    supplied: &'a std::collections::BTreeSet<String>,
}

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
    supplied: &'a std::collections::BTreeSet<String>,
}

#[derive(Serialize)]
struct RedisView<'a> {
    params: &'a CoordinationRedisParams,
    has_connection_target: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        serde_json::to_value(input.calls.iter().map(run_call).collect::<Vec<_>>()).or_value_error()
    }
}

fn run_call(call: &Call) -> Outcome {
    let value = evaluate(call).ok();
    Outcome {
        ok: value.is_some(),
        value,
    }
}

fn evaluate(call: &Call) -> Result<serde_json::Value, Box<HelperError>> {
    let body = serde_json::to_value(&call.body).or_value_error()?;
    match call.function {
        Function::Key => {
            serde_json::to_value(KeyRequest::from_value(&body).or_value_error()?).or_value_error()
        }
        Function::ResetSpend => {
            serde_json::to_value(ResetSpendRequest::from_value(&body).or_value_error()?)
                .or_value_error()
        }
        Function::CostEstimate => {
            serde_json::to_value(CostEstimateRequest::from_value(&body).or_value_error()?)
                .or_value_error()
        }
        Function::NewUser => new_user(&body),
        Function::UpdateUser => update_user(&body),
        Function::Customer => customer(&body),
        Function::RedisParams => redis_params(&body),
    }
}

fn new_user(body: &serde_json::Value) -> Result<serde_json::Value, Box<HelperError>> {
    let request = NewUserRequest::from_value(body).or_value_error()?;
    let view = NewUserView {
        user_email: request.user_email.as_ref(),
        user_alias: request.user_alias.as_ref(),
        user_role: request.user_role,
        max_budget: request.max_budget.as_ref(),
        teams: request.teams.as_ref(),
        auto_create_key: request.auto_create_key,
        send_invite_email: request.send_invite_email,
        sso_user_id: request.sso_user_id.as_ref(),
        organizations: request.organizations.as_ref(),
    };
    serde_json::to_value(view).or_value_error()
}

fn update_user(body: &serde_json::Value) -> Result<serde_json::Value, Box<HelperError>> {
    let request = UpdateUserRequest::from_value(body).or_value_error()?;
    let view = UpdateUserView {
        user_id: request.user_id.as_ref(),
        user_email: request.user_email.as_ref(),
        supplied: &request.fields.supplied,
    };
    serde_json::to_value(view).or_value_error()
}

fn customer(body: &serde_json::Value) -> Result<serde_json::Value, Box<HelperError>> {
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

fn redis_params(body: &serde_json::Value) -> Result<serde_json::Value, Box<HelperError>> {
    let params = CoordinationRedisParams::from_value(body).or_value_error()?;
    let view = RedisView {
        has_connection_target: params.has_connection_target(),
        params: &params,
    };
    serde_json::to_value(view).or_value_error()
}
