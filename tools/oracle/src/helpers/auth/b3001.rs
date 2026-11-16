mod fakes;
mod input;

use betterlite_auth::reservation_cost::input_cost::estimate_request_input_cost;
use betterlite_auth::reservation_cost::{
    EstimateRequest, EstimateSources, RouterModelInfo, count_request_input_tokens,
    estimate_request_max_cost,
};
use rust_decimal::prelude::ToPrimitive as _;

use self::fakes::{FakeModelInfo, FakeRouter, FakeTokens, NoDecoders};
use self::input::{Function, Input};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use std::fmt::Display;
use tokio::runtime::Builder;

// The estimate reads tokens from one of these body fields; B1695's tokenizer is not part of this helper.
const COUNTED_INPUT_FIELDS: [&str; 5] = ["messages", "prompt", "input", "query", "documents"];

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        reject_uncountable_estimate(input)?;
        let runtime = Builder::new_current_thread()
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
    let reused = input.token_counts.is_some();
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
    dispatch(input, &request, &sources).await
}

async fn dispatch(
    input: &Input,
    request: &EstimateRequest<'_>,
    sources: &EstimateSources<'_>,
) -> HelperResult {
    let counts = input.token_counts.as_ref();
    match input.function {
        Function::MaxCost => {
            let amount = estimate_request_max_cost(request, sources, counts)
                .await
                .map_err(|error| helper_error("ReservationCostError", error))?
                .and_then(|amount| amount.round_dp(9).to_f64());
            serde_json::to_value(amount).map_err(|error| helper_error("SerializationError", error))
        }
        Function::InputCost => {
            let amount = estimate_request_input_cost(request, sources, counts)
                .await
                .map_err(|error| helper_error("ReservationCostError", error))?
                .and_then(|amount| amount.round_dp(9).to_f64());
            serde_json::to_value(amount).map_err(|error| helper_error("SerializationError", error))
        }
        Function::Count => {
            let counted = count_request_input_tokens(request, sources)
                .await
                .map_err(|error| helper_error("ReservationCostError", error))?;
            serde_json::to_value(counted).map_err(|error| helper_error("SerializationError", error))
        }
    }
}

fn helper_error<M>(class: &str, message: M) -> Box<HelperError>
where
    M: Display,
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
