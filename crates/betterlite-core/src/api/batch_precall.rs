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
pub struct BatchPrecall<'precall, Stages, Stash, Refunds, Applier, Memory, Remote> {
    pub stages: &'precall Stages,
    pub stash: &'precall Stash,
    pub refunds: &'precall Refunds,
    pub applier: &'precall Applier,
    pub memory: &'precall Memory,
    pub remote: Option<&'precall Remote>,
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
        let refunds = RefundHook {
            stash: self.stash,
            refunds: self.refunds,
            applier: self.applier,
        };
        match &input.call {
            BatchPrecallCall::Create(call) => {
                let hook = CreateHook {
                    stages: self.stages,
                };
                Ok(BatchPrecallResult::Create(hook.run(call).await?))
            }
            BatchPrecallCall::Retrieve(call) => {
                let cache = RetrieveCache {
                    memory: self.memory,
                    remote: self.remote,
                    verbose_stdout: self.verbose_stdout,
                    clock: &self.clock,
                    limits: self.limits,
                };
                Ok(BatchPrecallResult::Retrieve(cache.run(call).await?))
            }
            BatchPrecallCall::CreateFailure => Ok(BatchPrecallResult::CreateFailure(
                refunds.create_failure().await,
            )),
            BatchPrecallCall::TerminalRetrieve(call) => Ok(BatchPrecallResult::TerminalRetrieve(
                refunds.terminal(call).await,
            )),
        }
    }
}
