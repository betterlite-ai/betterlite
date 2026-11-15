use std::mem::take;
use std::time::SystemTime;

use betterlite_logging::cost_breakdown::CostBreakdownTarget;
use rust_decimal::Decimal;

use super::Round;
use super::args::{CompletionArgs, CostCalculator};
use super::candidates::settle;
use super::error::{ChargeInputError, CompletionCostError};
use super::reading;
use super::usage::logging_usage;
use crate::cost::context::ProviderInference;
use crate::cost::image_dispatch::policy::ImagePricePolicy;
use crate::cost::prepare::{CompletionBillingInput, PreparedCompletion, ResponseView, prepare};
use crate::cost::realtime::RealtimeBilling;
use crate::cost::special::policy::ModelResolver;

// Each service-tier partition is its own charge, priced with the same logging context and summed.
pub(super) async fn charge_partitions<Resolver, ImagePolicy, Inference, Target, Clock>(
    services: &CostCalculator<'_, Resolver, ImagePolicy, Inference>,
    args: &CompletionArgs<'_>,
    partitions: &[RealtimeBilling],
    clock: &mut Clock,
    mut target: Option<&mut Target>,
) -> Result<Decimal, CompletionCostError<ImagePolicy::Error>>
where
    Resolver: ModelResolver + Sync,
    ImagePolicy: ImagePricePolicy + Sync,
    Inference: ProviderInference + Sync,
    Target: CostBreakdownTarget + Send,
    Clock: FnMut() -> SystemTime + Send,
{
    let moment = reading(clock)?;
    let mut total = Decimal::ZERO;
    for partition in partitions {
        let projected = logging_usage(&partition.usage)?;
        let view = ResponseView::Realtime {
            record: partition,
            usage: &projected,
        };
        let billing = CompletionBillingInput {
            response: Some(&view),
            ..args.billing
        };
        let PreparedCompletion::Attempts(boxed) = prepare(
            &billing,
            services.catalog,
            services.inference,
            services.counter,
        )
        .await?
        else {
            return Err(CompletionCostError::NestedPartitions);
        };
        let mut prepared = *boxed;
        let attempts = take(&mut prepared.attempts);
        let round = Round {
            args,
            billing: &billing,
            prepared: &prepared,
            moment,
            session: Some((partition.results.as_slice(), &partition.usage)),
        };
        let charge = settle(services, &round, attempts, clock, target.as_deref_mut())?;
        total = total
            .checked_add(charge)
            .ok_or(ChargeInputError::Overflow)?;
    }
    Ok(total)
}
