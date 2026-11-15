use std::time::UNIX_EPOCH;

use betterlite_core::cost::completion::completion_cost;
use betterlite_core::cost::response::response_cost_calculator;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::json;

use self::adapters::{Breakdown, failure};
use self::args::{build_parts, completion_args};
use self::frame::{build_adapters, build_calculator, build_stage};
use self::input::{Function, Input};
use self::views::response_view;

mod adapters;
mod args;
mod frame;
mod input;
mod views;

async fn charge(
    input: &Input,
    target: Option<&mut Breakdown>,
) -> Result<Decimal, Box<crate::helpers::HelperError>> {
    let stage = build_stage(input)?;
    let adapters = build_adapters(&stage, input);
    let calculator = build_calculator(&stage, &adapters);
    let parts = build_parts(input);
    let view = response_view(input, &parts);
    let args = completion_args(input, &parts, &view);
    let mut clock = || UNIX_EPOCH;
    let found = match input.function {
        Function::CompletionCost => completion_cost(&calculator, &args, &mut clock, target).await,
        Function::ResponseCostCalculator => {
            response_cost_calculator(&calculator, &args, input.cache_hit, &mut clock, target).await
        }
    };
    found.map_err(|error| failure("CompletionCostError", error.to_string()))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", error.to_string()))?;
        let mut target = Breakdown {
            refuse: input.refuse_breakdown == Some(true),
        };
        let cost = if input.refuse_breakdown == Some(true) {
            runtime.block_on(charge(input, Some(&mut target)))?
        } else {
            runtime.block_on(charge(input, None))?
        };
        Ok(json!({"cost": cost.to_f64()}))
    }
}
