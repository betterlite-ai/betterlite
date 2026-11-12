#[path = "pool/tests.rs"]
mod tests;

use std::sync::Arc;

use rust_decimal::Decimal;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::error::PromptInjectionError;
use super::similarity::check_user_input_similarity;

#[derive(Debug)]
pub struct HeuristicsPool {
    pub(crate) permits: Arc<Semaphore>,
}

impl HeuristicsPool {
    #[must_use]
    pub fn new(max_threads: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max_threads.min(Semaphore::MAX_PERMITS))),
        }
    }

    pub async fn check(
        &self,
        keywords: Arc<[String]>,
        input: String,
        threshold: Decimal,
    ) -> Result<bool, PromptInjectionError> {
        let permit = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_closed| PromptInjectionError::HeuristicsClosed)?;
        let mut tasks = JoinSet::new();
        tasks.spawn_blocking(move || {
            let _permit = permit;
            check_user_input_similarity(&keywords, &input, threshold)
        });
        let Some(joined) = tasks.join_next().await else {
            return Err(PromptInjectionError::HeuristicsClosed);
        };
        joined.map_err(|source| PromptInjectionError::HeuristicsWorker { source })
    }
}
