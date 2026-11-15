#![cfg(test)]

use std::convert::Infallible;

use betterlite_logging::cost_breakdown::CostBreakdownTarget;
use rust_decimal::Decimal;

use super::Rig;
use super::clock;
use crate::cost::completion::args::CompletionArgs;
use crate::cost::completion::error::CompletionCostError;
use crate::cost::response::response_cost_calculator;

// The response cost of one call against the rig's catalog, with the rig's clock and no response header unless the args carry one.
pub(crate) async fn respond<Target>(
    rig: &Rig,
    args: &CompletionArgs<'_>,
    cache_hit: Option<bool>,
    target: Option<&mut Target>,
) -> Result<Decimal, CompletionCostError<Infallible>>
where
    Target: CostBreakdownTarget + Send,
{
    let context = rig.context();
    response_cost_calculator(&rig.services(&context), args, cache_hit, &mut clock, target).await
}
