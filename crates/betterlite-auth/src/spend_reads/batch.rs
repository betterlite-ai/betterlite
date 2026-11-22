use std::future::{Future, ready};

use rust_decimal::Decimal;

use crate::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};
use crate::spend_counter_batch::scope::SpendCounterScope;
use crate::spend_reseed::cache::ActiveSpendBatch;

// the request-scoped batch B1718 owns, plus the forget a monotonic repair needs
pub trait SpendBatch: ActiveSpendBatch {
    fn forget(&self, counter_key: &str);
}

// a read with no request-scoped batch: no answers to give and nothing to forget
pub struct NoSpendBatch;

impl ActiveSpendBatch for NoSpendBatch {
    fn read(
        &self,
        _counter_key: &str,
    ) -> impl Future<Output = Result<Option<CounterValue>, SpendCounterBatchError>> + Send {
        ready(Ok(None))
    }

    fn record(&self, _counter_key: &str, _value: Decimal) {}
}

impl SpendBatch for NoSpendBatch {
    fn forget(&self, _counter_key: &str) {}
}

impl ActiveSpendBatch for SpendCounterScope {
    async fn read(
        &self,
        counter_key: &str,
    ) -> Result<Option<CounterValue>, SpendCounterBatchError> {
        self.read_spend_counter(counter_key).await
    }

    fn record(&self, counter_key: &str, value: Decimal) {
        self.record_spend_counter_value(counter_key, value);
    }
}

impl SpendBatch for SpendCounterScope {
    fn forget(&self, counter_key: &str) {
        self.forget_spend_counter(counter_key);
    }
}
