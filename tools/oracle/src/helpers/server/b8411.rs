use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::proxy_request_contracts::cost_estimate::CostEstimateRequest;
use betterlite_server::proxy_request_contracts::key::{KeyRequest, ResetSpendRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod customer;
mod new_user;
mod redis_params;
mod update_user;

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
    value: Option<Value>,
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

fn evaluate(call: &Call) -> Result<Value, Box<HelperError>> {
    let body = serde_json::to_value(&call.body).or_value_error()?;
    match call.function {
        Function::Key => record(KeyRequest::from_value(&body).or_value_error()?),
        Function::ResetSpend => record(ResetSpendRequest::from_value(&body).or_value_error()?),
        Function::CostEstimate => record(CostEstimateRequest::from_value(&body).or_value_error()?),
        Function::NewUser => new_user::answer(&body),
        Function::UpdateUser => update_user::answer(&body),
        Function::Customer => customer::answer(&body),
        Function::RedisParams => redis_params::answer(&body),
    }
}

fn record<T: Serialize>(value: T) -> Result<Value, Box<HelperError>> {
    serde_json::to_value(value).or_value_error()
}
