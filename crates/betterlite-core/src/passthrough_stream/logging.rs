use std::error::Error;

use async_trait::async_trait;
use betterlite_openai::passthrough::transformation::types::PassthroughTarget;

use super::spend_collection::BoxedCollector;

// The logging object a relayed stream reports through: LiteLLM's litellm_logging_obj flush methods and its model fields
#[async_trait]
pub trait SpendLogging<Logged: Send + 'static>: Send + Sync + 'static {
    type FlushError: Error + Send + Sync + 'static;

    fn spend_target(&self) -> PassthroughTarget<'_>;

    async fn async_flush_collected_chunks(
        &self,
        collector: BoxedCollector<Logged>,
    ) -> Result<(), Self::FlushError>;

    fn flush_collected_chunks(
        &self,
        collector: BoxedCollector<Logged>,
    ) -> Result<(), Self::FlushError>;
}

// LiteLLM's executor.submit: the blocking flush runs off the iterating thread, and a shut-down executor refuses it
pub trait SyncFlushExecutor: Send + Sync {
    type ScheduleError: Error + Send + 'static;

    fn submit(&self, job: Box<dyn FnOnce() + Send + 'static>) -> Result<(), Self::ScheduleError>;
}
