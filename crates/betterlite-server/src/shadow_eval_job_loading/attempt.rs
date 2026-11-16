use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;

use betterlite_db::shadow_tables::rows::{ShadowAttemptRow, ShadowTables};
use rust_decimal::Decimal;

use super::admission::job_spend_counter_key;
use super::jobs::ShadowEvalDirection;

pub const MAX_ERROR_CHARS: usize = 500;

pub type SpendFuture<'counter, Value, Failure> =
    Pin<Box<dyn Future<Output = Result<Value, Failure>> + Send + 'counter>>;

// the cross-pod job spend counter's write side, injected so tests own the sequencing
pub trait SpendCounter: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn add_spend<'counter>(
        &'counter self,
        counter_key: &'counter str,
        cost: Decimal,
    ) -> SpendFuture<'counter, (), Self::Error>;
}

// a reverse row shows which tier served the real traffic, a forward row the shadow's own
#[must_use]
pub fn attempt_tier(
    direction: ShadowEvalDirection,
    control_tier: Option<String>,
    shadow_tier: Option<String>,
) -> Option<String> {
    match direction {
        ShadowEvalDirection::Reverse => control_tier,
        ShadowEvalDirection::Forward => shadow_tier,
    }
}

// the counter is written before the row: a lost row still closes the job's budget gate
pub async fn record_attempt<Database, Counter>(
    database: Option<&Database>,
    counter: &Counter,
    mut attempt: ShadowAttemptRow,
) where
    Database: ShadowTables,
    Counter: SpendCounter,
{
    let eval_spend = attempt
        .judge_cost
        .saturating_add(attempt.shadow_cost)
        .saturating_add(attempt.shadow_classifier_cost);
    if eval_spend > Decimal::ZERO {
        let counter_key = job_spend_counter_key(&attempt.job_id);
        if let Err(error) = counter.add_spend(&counter_key, eval_spend).await {
            tracing::warn!(
                "shadow_eval: spend counter increment failed for {counter_key}: {error}"
            );
        }
    }
    let Some(database) = database else {
        return;
    };
    attempt.real_model = attempt.real_model.filter(|model| !model.is_empty());
    attempt.error = stored_error(attempt.error.take());
    if let Err(error) = database.insert_attempt(&attempt).await {
        tracing::debug!(
            "shadow_eval: attempt write failed for {}: {error}",
            attempt.request_id
        );
    }
}

// LiteLLM stores error[:500] and nothing for an empty message; the slice counts characters
fn stored_error(error: Option<String>) -> Option<String> {
    match error {
        Some(message) if !message.is_empty() => {
            Some(message.chars().take(MAX_ERROR_CHARS).collect())
        }
        _ => None,
    }
}
