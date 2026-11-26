use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};
use crate::spend_counter_batch::scope::SpendCounterScope;
use crate::spend_reseed::cache::ActiveSpendBatch;

// B1718's request-scoped batch as the writes use it, including the forget an invalidation sends
#[async_trait]
pub trait CounterBatch: Send + Sync {
    async fn read(&self, counter_key: &str)
    -> Result<Option<CounterValue>, SpendCounterBatchError>;

    fn record(&self, counter_key: &str, value: Decimal);

    fn forget(&self, counter_key: &str);
}

#[async_trait]
impl CounterBatch for SpendCounterScope {
    // read_batched_spend_counter binds the key first, so an open batch answers for it afterwards
    async fn read(
        &self,
        counter_key: &str,
    ) -> Result<Option<CounterValue>, SpendCounterBatchError> {
        self.read_batched_spend_counter(counter_key).await
    }

    fn record(&self, counter_key: &str, value: Decimal) {
        self.record_spend_counter_value(counter_key, value);
    }

    fn forget(&self, counter_key: &str) {
        self.forget_spend_counter(counter_key);
    }
}

// SpendCounterReseed warms through its own trait, so the dyn CounterBatch stands in for one there, and one impl for every batch region keeps the seeding chain provable in a Send future (rust-lang/rust#130113)
impl ActiveSpendBatch for Arc<dyn CounterBatch + '_> {
    async fn read(
        &self,
        counter_key: &str,
    ) -> Result<Option<CounterValue>, SpendCounterBatchError> {
        (**self).read(counter_key).await
    }

    fn record(&self, counter_key: &str, value: Decimal) {
        (**self).record(counter_key, value);
    }
}
