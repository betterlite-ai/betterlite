use std::mem::take;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_logging::cost_breakdown::CostBreakdownTarget;
use betterlite_models::usage_sum::Usage as PricingUsage;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::cost::context::ProviderInference;
use crate::cost::image_dispatch::policy::ImagePricePolicy;
use crate::cost::prepare::{
    CompletionBillingInput, PreparedBilling, PreparedCompletion, ResponseView, prepare,
};
use crate::cost::realtime::Event;
use crate::cost::special::policy::ModelResolver;
use args::{CompletionArgs, CostCalculator};
use candidates::settle;
use error::{ChargeInputError, CompletionCostError};
use partitions::charge_partitions;

pub mod args;
mod attempt;
pub mod breakdown;
pub mod error;
pub mod lookup;
pub mod resolver;
pub(crate) mod tests;
pub mod tool_source;
pub mod usage;

mod candidates;
mod partitions;

// What one prepared set is priced against: the call, the request as the partition sees it, the clock reading and any realtime session.
struct Round<'round> {
    args: &'round CompletionArgs<'round>,
    billing: &'round CompletionBillingInput<'round>,
    prepared: &'round PreparedBilling,
    moment: DateTime<Utc>,
    session: Option<(&'round [Event], &'round PricingUsage)>,
}

// One clock reading as a UTC moment; a time before the epoch or past the calendar is an error.
pub fn reading<Clock>(clock: &mut Clock) -> Result<DateTime<Utc>, ChargeInputError>
where
    Clock: FnMut() -> SystemTime,
{
    clock()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| {
            DateTime::from_timestamp(i64::try_from(since.as_secs()).ok()?, since.subsec_nanos())
        })
        .ok_or(ChargeInputError::Clock)
}

// Ports completion_cost: B1688 prepares the candidates, B1689 may finish early, and the token stage prices the rest.
pub async fn completion_cost<Resolver, ImagePolicy, Inference, Target, Clock>(
    services: &CostCalculator<'_, Resolver, ImagePolicy, Inference>,
    args: &CompletionArgs<'_>,
    clock: &mut Clock,
    target: Option<&mut Target>,
) -> Result<Decimal, CompletionCostError<ImagePolicy::Error>>
where
    Resolver: ModelResolver + Sync,
    ImagePolicy: ImagePricePolicy + Sync,
    Inference: ProviderInference + Sync,
    Target: CostBreakdownTarget + Send,
    Clock: FnMut() -> SystemTime + Send,
{
    let outcome = prepare(
        &args.billing,
        services.catalog,
        services.inference,
        services.counter,
    )
    .await?;
    match outcome {
        PreparedCompletion::WebsocketPartitions(partitions) => {
            charge_partitions(services, args, &partitions, clock, target).await
        }
        PreparedCompletion::Attempts(boxed) => {
            let mut prepared = *boxed;
            let attempts = take(&mut prepared.attempts);
            let session = match args.billing.response {
                Some(ResponseView::Realtime { record, .. }) => {
                    Some((record.results.as_slice(), &record.usage))
                }
                _ => None,
            };
            let round = Round {
                args,
                billing: &args.billing,
                prepared: &prepared,
                moment: reading(clock)?,
                session,
            };
            settle(services, &round, attempts, clock, target)
        }
    }
}
