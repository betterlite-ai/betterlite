mod fakes;
mod input;

use betterlite_auth::reservation_cost::error::ReservationCostError;
use betterlite_auth::reservation_cost::{
    EstimateRequest, EstimateSources, RouterModelInfo, count_request_input_tokens,
    estimate_request_input_cost, estimate_request_max_cost,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use self::fakes::{FakeModelInfo, FakeRouter, FakeTokens, NoDecoders};
use self::input::{Function, Input};
use crate::helpers::{Helper, HelperError, HelperResult};

// The estimate reads tokens from one of these body fields; B1695's tokenizer is not part of this helper.
const COUNTED_INPUT_FIELDS: [&str; 5] = ["messages", "prompt", "input", "query", "documents"];

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        reject_uncountable_estimate(input)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| helper_error("RuntimeError", error))?;
        runtime.block_on(estimate(input))
    }
}

fn reject_uncountable_estimate(input: &Input) -> Result<(), Box<HelperError>> {
    let countable = COUNTED_INPUT_FIELDS
        .iter()
        .any(|field| input.request.contains_key(*field));
    if !countable {
        return Ok(());
    }
    let reused = input.input_token_counts.is_some();
    if reused && !matches!(input.function, Function::Count) {
        return Ok(());
    }
    Err(helper_error(
        "UnsupportedInput",
        "the case must supply input_token_counts: the tokenizer is outside this helper",
    ))
}

async fn estimate(input: &Input) -> HelperResult {
    let model_info = FakeModelInfo {
        entries: &input.model_cost,
    };
    let router = FakeRouter {
        entries: &input.model_cost,
        deployments: &input.model_list,
    };
    let router_info: &dyn RouterModelInfo = &router;
    let tokens = FakeTokens;
    let decoders = NoDecoders;
    let sources = EstimateSources {
        router: Some(router_info),
        model_info: &model_info,
        tokens: &tokens,
        catalog: None,
        decoders: &decoders,
    };
    let request = EstimateRequest {
        body: &input.request,
        route: input.route.as_str(),
    };
    let counts = input.input_token_counts.as_ref();
    match input.function {
        Function::MaxCost => estimate_request_max_cost(&request, &sources, counts)
            .await
            .map_err(cost_error)
            .and_then(defined_cost),
        Function::InputCost => estimate_request_input_cost(&request, &sources, counts)
            .await
            .map_err(cost_error)
            .and_then(defined_cost),
        Function::Count => count_request_input_tokens(&request, &sources)
            .await
            .map_err(cost_error)
            .and_then(serialize),
    }
}

fn defined_cost(cost: Option<Decimal>) -> HelperResult {
    let amount = cost.map(|amount| amount.round_dp(9).to_f64());
    serialize(amount)
}

fn serialize<T>(value: T) -> HelperResult
where
    T: serde::Serialize,
{
    serde_json::to_value(value).map_err(|error| helper_error("SerializationError", error))
}

fn cost_error(error: ReservationCostError) -> Box<HelperError> {
    helper_error("ReservationCostError", error)
}

fn helper_error<M>(class: &str, message: M) -> Box<HelperError>
where
    M: std::fmt::Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
