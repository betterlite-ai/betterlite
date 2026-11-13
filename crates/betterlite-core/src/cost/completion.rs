use std::mem::take;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_logging::cost_breakdown::CostBreakdownTarget;
use betterlite_models::usage_sum::Usage as PricingUsage;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::cost::context::ProviderInference;
use crate::cost::image_dispatch::policy::ImagePricePolicy;
use crate::cost::prepare::{
    AttemptOutcome, CompletionBillingInput, PreparedBilling, PreparedBillingAttempt,
    PreparedCompletion, ResponseView, prepare,
};
use crate::cost::realtime::{Event, RealtimeBilling};
use crate::cost::special::policy::ModelResolver;
use crate::cost::special::request::{RealtimeSession, SpecialOutcome};
use crate::cost::special::special_charge;
use args::{CompletionArgs, CostCalculator};
use attempt::attempt_charge;
use error::{ChargeInputError, CompletionCostError};
use usage::logging_usage;

pub mod args;
mod attempt;
pub mod breakdown;
pub mod error;
pub mod lookup;
pub mod resolver;
pub(crate) mod tests;
pub mod tool_source;
pub mod usage;

// What one prepared set is priced against: the call, the request as the partition sees it, the clock reading and any realtime session.
struct Round<'round> {
    args: &'round CompletionArgs<'round>,
    billing: &'round CompletionBillingInput<'round>,
    prepared: &'round PreparedBilling,
    moment: DateTime<Utc>,
    session: Option<(&'round [Event], &'round PricingUsage)>,
}

// One clock reading as a UTC moment; a time before the epoch or past the calendar is an error.
pub fn reading<Clock: FnMut() -> SystemTime>(
    clock: &mut Clock,
) -> Result<DateTime<Utc>, ChargeInputError> {
    clock()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| {
            DateTime::from_timestamp(i64::try_from(since.as_secs()).ok()?, since.subsec_nanos())
        })
        .ok_or(ChargeInputError::Clock)
}

// One candidate name: B1689 may finish the charge early, a failed preparation is raised as it is, and anything else goes to the token stage.
fn one_attempt<Resolver, ImagePolicy, Inference, Target, Clock>(
    services: &CostCalculator<'_, Resolver, ImagePolicy, Inference>,
    round: &Round<'_>,
    attempt: PreparedBillingAttempt,
    clock: &mut Clock,
    mut target: Option<&mut Target>,
) -> Result<Decimal, CompletionCostError<ImagePolicy::Error>>
where
    Resolver: ModelResolver,
    ImagePolicy: ImagePricePolicy,
    Target: CostBreakdownTarget,
    Clock: FnMut() -> SystemTime,
{
    let realtime = round.session.map(|(results, usage)| RealtimeSession {
        results,
        usage: Some(usage),
    });
    let request =
        round
            .args
            .charge
            .special_request(round.billing, round.prepared, round.moment, realtime);
    match special_charge(
        &services.special,
        &request,
        attempt,
        clock,
        target.as_deref_mut(),
    )? {
        SpecialOutcome::Charged(charge) => Ok(charge),
        // special_charge hands an attempt back unchanged only when its preparation failed.
        SpecialOutcome::Failed(failed) => match failed.outcome {
            AttemptOutcome::Failed(error) => Err(CompletionCostError::Prepare(error)),
            AttemptOutcome::Ready(_) => Err(CompletionCostError::NoCandidates),
        },
        SpecialOutcome::Continue(stage) => attempt_charge(
            services,
            round.args,
            round.prepared,
            *stage,
            round.moment,
            target,
        ),
    }
}

// Walks the candidate names of one prepared set: the first charge wins, and the last attempt's failure is the one raised.
fn settle<Resolver, ImagePolicy, Inference, Target, Clock>(
    services: &CostCalculator<'_, Resolver, ImagePolicy, Inference>,
    round: &Round<'_>,
    attempts: Vec<PreparedBillingAttempt>,
    clock: &mut Clock,
    mut target: Option<&mut Target>,
) -> Result<Decimal, CompletionCostError<ImagePolicy::Error>>
where
    Resolver: ModelResolver,
    ImagePolicy: ImagePricePolicy,
    Target: CostBreakdownTarget,
    Clock: FnMut() -> SystemTime,
{
    let last = attempts.len().saturating_sub(1);
    for (index, attempt) in attempts.into_iter().enumerate() {
        let candidate = attempt.model.clone();
        match one_attempt(services, round, attempt, clock, target.as_deref_mut()) {
            Ok(charge) => return Ok(charge),
            Err(error) if index == last => return Err(error),
            Err(error) => {
                tracing::debug!(model = ?candidate, %error, "Error calculating cost for model");
            }
        }
    }
    Err(CompletionCostError::NoCandidates)
}

// Each service-tier partition is its own charge, priced with the same logging context and summed.
async fn charge_partitions<Resolver, ImagePolicy, Inference, Target, Clock>(
    services: &CostCalculator<'_, Resolver, ImagePolicy, Inference>,
    args: &CompletionArgs<'_>,
    partitions: &[RealtimeBilling],
    clock: &mut Clock,
    mut target: Option<&mut Target>,
) -> Result<Decimal, CompletionCostError<ImagePolicy::Error>>
where
    Resolver: ModelResolver,
    ImagePolicy: ImagePricePolicy,
    Inference: ProviderInference + Sync,
    Target: CostBreakdownTarget,
    Clock: FnMut() -> SystemTime,
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
        let PreparedCompletion::Attempts(prepared) = prepare(
            &billing,
            services.catalog,
            services.inference,
            services.counter,
        )
        .await?
        else {
            return Err(CompletionCostError::NestedPartitions);
        };
        let mut prepared = *prepared;
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

// Ports completion_cost: B1688 prepares the candidates, B1689 may finish early, and the token stage prices the rest.
pub async fn completion_cost<Resolver, ImagePolicy, Inference, Target, Clock>(
    services: &CostCalculator<'_, Resolver, ImagePolicy, Inference>,
    args: &CompletionArgs<'_>,
    clock: &mut Clock,
    target: Option<&mut Target>,
) -> Result<Decimal, CompletionCostError<ImagePolicy::Error>>
where
    Resolver: ModelResolver,
    ImagePolicy: ImagePricePolicy,
    Inference: ProviderInference + Sync,
    Target: CostBreakdownTarget,
    Clock: FnMut() -> SystemTime,
{
    let prepared = prepare(
        &args.billing,
        services.catalog,
        services.inference,
        services.counter,
    )
    .await?;
    match prepared {
        PreparedCompletion::WebsocketPartitions(partitions) => {
            charge_partitions(services, args, &partitions, clock, target).await
        }
        PreparedCompletion::Attempts(prepared) => {
            let mut prepared = *prepared;
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
