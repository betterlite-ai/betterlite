use tokio::task::{JoinError, JoinSet};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("the credential worker could not run: {source}")]
    Failed {
        #[source]
        source: JoinError,
    },
    #[error("the credential worker stopped without running")]
    Stopped,
}

pub async fn run_blocking<T, F>(work: F) -> Result<T, WorkerError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(work);
    match tasks.join_next().await {
        Some(Ok(value)) => Ok(value),
        Some(Err(source)) => Err(WorkerError::Failed { source }),
        None => Err(WorkerError::Stopped),
    }
}
