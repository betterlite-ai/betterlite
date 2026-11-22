use betterlite_db::spend_rows::rows::SpendCounterRows as DbSpendCounterRows;
use rust_decimal::Decimal;

use super::SpendReads;
use super::batch::SpendBatch;
use super::cache::SpendReadCache;
use super::request::SpendReadRequest;
use crate::spend_reseed::rows::SpendCounterRows;

// what the authoritative recheck settled for a counter that is below its budget
pub(super) enum Recheck {
    // the authoritative floor replaced the cache estimate
    Raised(Decimal),
    // the authoritative source answered and the estimate stands
    Verified,
    // no authoritative answer, so the estimate's earlier verification is unchanged
    Unchanged,
}

// the database recheck LiteLLM runs before enforcing a budget on a possibly stale counter
pub(super) async fn recheck_spend<Rows, Cache, Batch>(
    reads: &SpendReads<'_, Rows, Cache, Batch>,
    request: &SpendReadRequest<'_>,
    current: Decimal,
) -> Recheck
where
    Rows: SpendCounterRows + DbSpendCounterRows,
    Cache: SpendReadCache,
    Batch: SpendBatch,
{
    let stale_low = request.fallback_spend > current;
    if !stale_low && request.window.is_none() && !reads.fail_closed_budget_enforcement {
        return Recheck::Unchanged;
    }
    match reads
        .authoritative_floor_spend(request.counter_key, request.window.as_ref())
        .await
    {
        Some(floor) if floor > current => {
            reads
                .repair_stale_spend_counter(request.counter_key, floor)
                .await;
            Recheck::Raised(floor)
        }
        Some(_) => Recheck::Verified,
        None if stale_low => Recheck::Raised(request.fallback_spend),
        None => Recheck::Unchanged,
    }
}
