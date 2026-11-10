use std::fmt::Display;

use betterlite_core::cost::usage::provider_cost::{
    hidden_response_cost, without_provider_stated_cost,
};
use betterlite_core::cost::usage::tiers::{resolve_billable_service_tier, served_service_tier};
use betterlite_core::cost::usage::{BillingResponse, ResponseKind, get_usage_object};
use serde::Deserialize;
use serde_json::{Value, json};

use model::{decimal_value, optional_mapping, optional_usage, parse_usage, usage_projection};
mod model;

type Failure = Box<crate::helpers::HelperError>;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    response: Option<ResponseInput>,
    usage: Option<Value>,
    hidden_params: Option<Value>,
    requested_tier: Option<String>,
    served_tier: Option<String>,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    GetUsageObject,
    WithoutProviderStatedCost,
    HiddenResponseCost,
    ServedServiceTier,
    ResolveBillableServiceTier,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseInput {
    kind: ResponseKind,
    usage: Option<UsageSpec>,
    service_tier: Option<String>,
    hidden_params: Option<Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageSpec {
    kind: UsageKindInput,
    value: Option<Value>,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UsageKindInput {
    Absent,
    Null,
    Chat,
    Responses,
    Transcription,
    DecisionsIr,
    #[serde(rename = "decisions_openai")]
    DecisionsOpenAi,
    Interactions,
    Mapping,
}
pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let outcomes = input.calls.iter().map(run_call).collect::<Vec<_>>();
        Ok(Value::Array(outcomes))
    }
}
fn run_call(call: &Call) -> Value {
    match execute(call) {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(error) => json!({"ok": false, "error": {"class": error.class}}),
    }
}
fn execute(call: &Call) -> Result<Value, Failure> {
    match call.function {
        Function::GetUsageObject => get_usage(call),
        Function::WithoutProviderStatedCost => {
            let Some(usage) = optional_usage(call)? else {
                return Ok(Value::Null);
            };
            let cleared = without_provider_stated_cost(&usage);
            Ok(usage_projection(Some(cleared.as_ref())))
        }
        Function::HiddenResponseCost => {
            let hidden = optional_mapping(call.hidden_params.as_ref())?;
            let cost = hidden_response_cost(hidden.as_ref()).map_err(failure)?;
            Ok(cost.map_or(Value::Null, |amount| decimal_value(&amount)))
        }
        Function::ServedServiceTier => served_tier(call),
        Function::ResolveBillableServiceTier => {
            let tier = resolve_billable_service_tier(
                call.requested_tier.as_deref(),
                call.served_tier.as_deref(),
            );
            Ok(Value::from(tier.as_deref()))
        }
    }
}
fn get_usage(call: &Call) -> Result<Value, Failure> {
    let spec = required_response(call)?;
    let owned = parse_usage(spec.usage.as_ref())?;
    let hidden = optional_mapping(spec.hidden_params.as_ref())?;
    let usage = get_usage_object(&BillingResponse {
        kind: spec.kind,
        usage: owned.borrowed(),
        service_tier: spec.service_tier.as_deref(),
        hidden_params: hidden.as_ref(),
    })
    .map_err(failure)?;
    Ok(usage_projection(usage.as_ref()))
}

// The source reads the served tier from the response and the normalized usage, never from the response usage object.
fn served_tier(call: &Call) -> Result<Value, Failure> {
    let input = call
        .response
        .as_ref()
        .ok_or_else(|| failure("response is required"))?;
    Ok(Value::from(
        served_service_tier(
            input.service_tier.as_deref(),
            optional_mapping(input.hidden_params.as_ref())?.as_ref(),
            optional_usage(call)?.as_ref(),
        )
        .as_deref(),
    ))
}
fn required_response(call: &Call) -> Result<&ResponseInput, Failure> {
    call.response
        .as_ref()
        .ok_or_else(|| failure("response is required"))
}
fn failure<Error>(error: Error) -> Failure
where
    Error: Display,
{
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
