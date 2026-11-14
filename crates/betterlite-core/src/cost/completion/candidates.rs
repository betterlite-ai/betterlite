use std::time::SystemTime;

use betterlite_logging::cost_breakdown::CostBreakdownTarget;
use rust_decimal::Decimal;

use super::Round;
use super::args::CostCalculator;
use super::attempt::one_attempt;
use super::error::CompletionCostError;
use crate::cost::image_dispatch::policy::ImagePricePolicy;
use crate::cost::prepare::PreparedBillingAttempt;
use crate::cost::special::policy::ModelResolver;

// Walks the candidate names of one prepared set: the first charge wins, and the last attempt's failure is the one raised.
pub(super) fn settle<Resolver, ImagePolicy, Inference, Target, Clock>(
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
