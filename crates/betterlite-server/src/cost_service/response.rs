use std::time::SystemTime;

use betterlite_core::cost::completion::error::CompletionCostError;
use betterlite_core::cost::response::response_cost_calculator;
use betterlite_logging::cost_breakdown::CostBreakdownTarget;
use betterlite_provider::pricing::grounding::GroundingFormulas;
use betterlite_provider::pricing::image::error::ImagePolicyError;
use rust_decimal::Decimal;

use super::{Call, CostService};

// response_cost_calculator with every collaborator the cost service owns.
pub async fn response_cost<Formulas, Target, Clock>(
    service: &CostService<Formulas>,
    call: &Call<'_, '_>,
    cache_hit: Option<bool>,
    clock: &mut Clock,
    target: Option<&mut Target>,
) -> Result<Decimal, CompletionCostError<ImagePolicyError>>
where
    Formulas: GroundingFormulas + Copy + Send + Sync + 'static,
    Target: CostBreakdownTarget + Send,
    Clock: FnMut() -> SystemTime + Send,
{
    let bound = service.bound(call.args.billing.model, clock)?;
    let calculator = service.calculator(&bound, call.counter);
    response_cost_calculator(&calculator, call.args, cache_hit, clock, target).await
}
