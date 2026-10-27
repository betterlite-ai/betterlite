use tokio::task::JoinSet;

use super::error::SandboxError;
use crate::errors::HarnessError;

pub(super) async fn blocking<Job, Output>(job: Job) -> Result<Output, HarnessError>
where
    Job: FnOnce() -> Result<Output, HarnessError> + Send + 'static,
    Output: Send + 'static,
{
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(job);
    match tasks.join_next().await {
        Some(Ok(result)) => result,
        Some(Err(source)) => Err(HarnessError::from(SandboxError::Worker { source })),
        None => Err(HarnessError::from(SandboxError::NoResult)),
    }
}
