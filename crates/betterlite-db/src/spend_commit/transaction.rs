use std::future::Future;
use std::time::Duration;

use thiserror::Error;
use tokio::time::timeout;

// _spend_update_tx: prisma gives one interactive transaction sixty seconds before it abandons it.
pub const SPEND_TRANSACTION_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Error)]
pub enum SpendTransactionError<E> {
    #[error("the spend update transaction failed")]
    Work(#[source] E),
    #[error("the spend update transaction timed out after {seconds}s")]
    Timeout { seconds: u64 },
}

#[derive(Debug, Clone, Copy)]
pub struct SpendDeadline {
    pub duration: Duration,
}

impl SpendDeadline {
    pub const SOURCE: Self = Self {
        duration: SPEND_TRANSACTION_TIMEOUT,
    };

    // The statements and the commit share the source's one deadline, and an expired attempt is reported as its own failure.
    pub async fn run<T, E, Work>(&self, work: Work) -> Result<T, SpendTransactionError<E>>
    where
        Work: Future<Output = Result<T, E>>,
    {
        match timeout(self.duration, work).await {
            Ok(result) => result.map_err(SpendTransactionError::Work),
            Err(_elapsed) => Err(SpendTransactionError::Timeout {
                seconds: self.duration.as_secs(),
            }),
        }
    }
}
