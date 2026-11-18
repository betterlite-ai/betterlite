// B2316 builds this state machine in spend_counter_batch; this module reuses it whole
pub use crate::spend_counter_batch::batch::{
    CounterValue as SpendCounterValue, SpendCounterBatch, SpendCounterBatchError,
};
