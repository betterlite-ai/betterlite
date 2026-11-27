mod cache;
mod create;
mod fields;
pub mod input;
pub mod ports;
mod refunds;
pub mod result;
mod tests;

use crate::api::api_boundaries::ApiLimits;
use crate::api::batch_limit_counters::records::EpochClock;

use cache::RetrieveCache;
use create::CreateHook;
use input::{BatchPrecallCall, BatchPrecallInput};
use ports::{
    BatchPrecallMemory, BatchPrecallRemote, BatchPrecallStages, BatchPrecallStash, BatchRefundSink,
    BatchTokenApplier,
};
use refunds::RefundHook;
use result::BatchPrecallResult;

// The one failure that leaves the hook's swallowed-error policy: a request field the typed record cannot hold
#[derive(Debug, thiserror::Error)]
pub enum BatchPrecallError {
    #[error("the batch request field {0} has the wrong type")]
    Field(&'static str),
    #[error("the request metadata is missing")]
    MetadataMissing,
    #[error("the request metadata is not an object")]
    MetadataNotObject,
}

// The two batch hooks over their injected dependencies, plus the create-failure and terminal-retrieve refunds the stash carries
pub struct BatchPrecall<'a, Stages, Stash, Refunds, Applier, Memory, Remote> {
    pub stages: &'a Stages,
    pub stash: &'a Stash,
    pub refunds: &'a Refunds,
    pub applier: &'a Applier,
    pub memory: &'a Memory,
    pub remote: Option<&'a Remote>,
    pub verbose_stdout: bool,
    pub clock: EpochClock,
    pub limits: ApiLimits,
}

impl<Stages, Stash, Refunds, Applier, Memory, Remote>
    BatchPrecall<'_, Stages, Stash, Refunds, Applier, Memory, Remote>
where
    Stages: BatchPrecallStages,
    Stash: BatchPrecallStash,
    Refunds: BatchRefundSink,
    Applier: BatchTokenApplier,
    Memory: BatchPrecallMemory,
    Remote: BatchPrecallRemote,
{
    pub async fn run(
        &self,
        input: &BatchPrecallInput,
    ) -> Result<BatchPrecallResult, BatchPrecallError> {
        match &input.call {
            BatchPrecallCall::Create(call) => {
                let result = CreateHook {
                    stages: self.stages,
                }
                .run(call)
                .await?;
                Ok(BatchPrecallResult::Create(result))
            }
            BatchPrecallCall::Retrieve(call) => {
                let cache = RetrieveCache {
                    memory: self.memory,
                    remote: self.remote,
                    verbose_stdout: self.verbose_stdout,
                    clock: &self.clock,
                    limits: self.limits,
                };
                let result = cache.run(call).await?;
                Ok(BatchPrecallResult::Retrieve(result))
            }
            BatchPrecallCall::CreateFailure => {
                let result = RefundHook {
                    stash: self.stash,
                    refunds: self.refunds,
                    applier: self.applier,
                }
                .create_failure()
                .await;
                Ok(BatchPrecallResult::CreateFailure(result))
            }
            BatchPrecallCall::TerminalRetrieve(call) => {
                let result = RefundHook {
                    stash: self.stash,
                    refunds: self.refunds,
                    applier: self.applier,
                }
                .terminal(call)
                .await;
                Ok(BatchPrecallResult::TerminalRetrieve(result))
            }
        }
    }
}
