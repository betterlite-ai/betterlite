use std::future::Future;

use tokio::runtime::Builder;

use super::failure::Failure;

pub(crate) fn block_on<Job>(job: Job) -> Result<Job::Output, Failure>
where
    Job: Future,
{
    let runtime = Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()?;
    Ok(runtime.block_on(job))
}
