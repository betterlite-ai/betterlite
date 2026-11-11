use super::error::SigningError;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::dispatcher::{get_default, with_default};
use tracing::{Dispatch, Span};
pub const AWS_SIGNING_MAX_THREADS: usize = 16;
#[derive(Debug, Clone)]
pub struct AwsSigningExecutor {
    permits: Arc<Semaphore>,
}
impl Default for AwsSigningExecutor {
    fn default() -> Self {
        Self {
            permits: Arc::new(Semaphore::new(AWS_SIGNING_MAX_THREADS)),
        }
    }
}
impl AwsSigningExecutor {
    pub async fn run<T, F>(&self, sign: F) -> Result<T, SigningError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let span = Span::current();
        let dispatch = get_default(Dispatch::clone);
        let permit = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_closed| SigningError::ExecutorClosed)?;
        let mut tasks = JoinSet::new();
        tasks.spawn_blocking(move || {
            let _permit = permit;
            with_default(&dispatch, || span.in_scope(sign))
        });
        tasks
            .join_next()
            .await
            .ok_or(SigningError::ExecutorClosed)?
            .map_err(SigningError::Join)
    }
}
pub async fn sign_request_off_loop_if_aws<T, F>(
    executor: &AwsSigningExecutor,
    signs_with_aws: bool,
    sign: F,
) -> Result<T, SigningError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    if signs_with_aws {
        executor.run(sign).await
    } else {
        Ok(sign())
    }
}
