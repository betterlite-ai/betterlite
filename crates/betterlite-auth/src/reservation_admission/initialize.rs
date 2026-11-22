use crate::budget_counter::BudgetCounter;
use crate::spend_writes::error::SpendWriteError;

use super::counters::AdmissionCounters;
use super::error::{AdmissionError, unavailable};

// the three answers _initialize_reservation_counter gives: loaded, unloadable without an error, or a failed seed
enum SeedAnswer {
    Loaded,
    Unavailable,
    Failed(SpendWriteError),
}

// _initialize_reservation_counters: the counters whose value is loaded, in order, one that cannot be loaded skipped unless enforcement is strict
pub(crate) async fn initialize_counters<'a, C>(
    counters: &C,
    budget_counters: &'a [BudgetCounter],
    fail_closed_budget_enforcement: bool,
) -> Result<Vec<&'a BudgetCounter>, AdmissionError>
where
    C: AdmissionCounters,
{
    let mut loaded = Vec::new();
    for counter in budget_counters {
        if counter_loaded(counters, counter, fail_closed_budget_enforcement).await? {
            loaded.push(counter);
        }
    }
    Ok(loaded)
}

// _reservation_counter_loaded: a failed initialization skips the counter, or rejects the request under strict enforcement
async fn counter_loaded<C>(
    counters: &C,
    counter: &BudgetCounter,
    fail_closed_budget_enforcement: bool,
) -> Result<bool, AdmissionError>
where
    C: AdmissionCounters,
{
    match seed(counters, counter).await {
        SeedAnswer::Loaded => Ok(true),
        SeedAnswer::Unavailable => {
            tracing::warn!(
                counter_key = counter.counter_key,
                "Skipping budget reservation because window spend could not be loaded"
            );
            reject_or_skip(counter, fail_closed_budget_enforcement)
        }
        SeedAnswer::Failed(error) => {
            tracing::warn!(
                counter_key = counter.counter_key,
                %error,
                "Skipping budget reservation because spend counter reservation failed"
            );
            counters.invalidate(&counter.counter_key).await;
            reject_or_skip(counter, fail_closed_budget_enforcement)
        }
    }
}

// _raise_reservation_unavailable when enforcement is strict, else the counter is simply skipped
fn reject_or_skip(
    counter: &BudgetCounter,
    fail_closed_budget_enforcement: bool,
) -> Result<bool, AdmissionError> {
    if fail_closed_budget_enforcement {
        return Err(unavailable(&counter.counter_key));
    }
    Ok(false)
}

// _initialize_reservation_counter: the source cache key path, the window path, or a counter with nothing to load
async fn seed<C>(counters: &C, counter: &BudgetCounter) -> SeedAnswer
where
    C: AdmissionCounters,
{
    let Some(source_cache_key) = &counter.source_cache_key else {
        if counter.spend_log_entity_id.is_none() || counter.window_start.is_none() {
            return SeedAnswer::Loaded;
        }
        return match counters.seed_window(counter).await {
            Ok(true) => SeedAnswer::Loaded,
            Ok(false) => SeedAnswer::Unavailable,
            Err(error) => SeedAnswer::Failed(error),
        };
    };
    match counters
        .seed_source(&counter.counter_key, source_cache_key)
        .await
    {
        Ok(()) => SeedAnswer::Loaded,
        Err(error) => SeedAnswer::Failed(error),
    }
}
