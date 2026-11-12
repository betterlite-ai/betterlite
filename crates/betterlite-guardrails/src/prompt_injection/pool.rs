#[path = "pool/tests.rs"]
mod tests;

use std::sync::Arc;
use std::time::{Duration, Instant};

use rust_decimal::Decimal;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::error::PromptInjectionError;
use super::similarity::check_user_input_similarity;

#[derive(Debug)]
pub struct HeuristicsPool {
    pub(crate) permits: Arc<Semaphore>,
    timeout: Duration,
}

impl HeuristicsPool {
    #[must_use]
    pub fn new(max_threads: usize, timeout: Duration) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max_threads.min(Semaphore::MAX_PERMITS))),
            timeout,
        }
    }

    pub async fn check(
        &self,
        keywords: Arc<[String]>,
        input: String,
        threshold: Decimal,
    ) -> Result<bool, PromptInjectionError> {
        let prompt_length = input.chars().count();
        let started = Instant::now();
        let permit = match timeout(self.timeout, Arc::clone(&self.permits).acquire_owned()).await {
            Err(_elapsed) => {
                tracing::warn!(
                    elapsed = ?started.elapsed(),
                    prompt_length,
                    "the prompt injection heuristics check timed out waiting for a permit"
                );
                return Err(PromptInjectionError::HeuristicsTimeout);
            }
            Ok(Err(_closed)) => return Err(PromptInjectionError::HeuristicsClosed),
            Ok(Ok(permit)) => permit,
        };
        let deadline = started.checked_add(self.timeout);
        let mut tasks = JoinSet::new();
        tasks.spawn_blocking(move || {
            let _permit = permit;
            check_user_input_similarity(&keywords, &input, threshold, deadline)
        });
        let Some(joined) = tasks.join_next().await else {
            return Err(PromptInjectionError::HeuristicsClosed);
        };
        match joined {
            Err(source) => Err(PromptInjectionError::HeuristicsWorker { source }),
            Ok(Err(PromptInjectionError::HeuristicsTimeout)) => {
                tracing::warn!(
                    elapsed = ?started.elapsed(),
                    prompt_length,
                    "the prompt injection heuristics scan timed out"
                );
                Err(PromptInjectionError::HeuristicsTimeout)
            }
            Ok(result) => result,
        }
    }
}
