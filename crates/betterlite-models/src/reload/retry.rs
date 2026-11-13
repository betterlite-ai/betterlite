use std::sync::Arc;

use crate::catalog::CatalogError;
use crate::fetch_policy::client::CatalogClient;
use crate::fetch_policy::outcome::{
    AttemptOutcome, FetchResult, Unavailable, classify_fetch_error, classify_fetch_response,
};
use crate::fetch_policy::retry::{CatalogSleeper, JitterSource, NextStep, next_retry_step};
use crate::reload::refresh;
use crate::reload::{CatalogService, RetryOutcome, RetryTask};

impl<C, S, J> RetryTask<C, S, J>
where
    C: CatalogClient,
    S: CatalogSleeper,
    J: JitterSource,
{
    pub async fn run<F>(mut self, service: Arc<CatalogService<F>>) -> RetryOutcome
    where
        F: Fn() -> Result<Vec<u8>, CatalogError> + Send + Sync + 'static,
    {
        let first_wait = match next_retry_step(
            AttemptOutcome::Retryable(self.first_outcome.clone()),
            1,
            self.limits.max_attempts,
            &mut self.jitter,
        ) {
            NextStep::Retry(seconds) => seconds,
            NextStep::Done(_) => return RetryOutcome::Kept,
        };
        self.sleeper.sleep(first_wait).await;
        match self.fetch_remaining().await {
            FetchResult::Unavailable(unavailable) => {
                tracing::warn!(
                    url = %self.request.url,
                    reason = %unavailable.reason,
                    "model cost map retry failed; keeping the loaded map"
                );
                RetryOutcome::Kept
            }
            FetchResult::Reloaded(fetched) => {
                match refresh::adopt(&service, &self.request.url, &fetched, (service.clock)()) {
                    Ok(snapshot) => RetryOutcome::Adopted(snapshot),
                    Err(error) => {
                        tracing::warn!(
                            url = %self.request.url,
                            %error,
                            "model cost map retry was not adopted; keeping the loaded map"
                        );
                        RetryOutcome::Kept
                    }
                }
            }
        }
    }

    async fn fetch_remaining(&mut self) -> FetchResult {
        let mut attempt = 2;
        loop {
            if attempt > self.limits.max_attempts {
                return FetchResult::Unavailable(Unavailable {
                    reason: "model cost map fetch failed".to_owned(),
                });
            }
            let outcome = match self.client.get_async(&self.request).await {
                Ok(response) => classify_fetch_response(response, &self.request.url),
                Err(error) => classify_fetch_error(&error, &self.request.url),
            };
            match next_retry_step(outcome, attempt, self.limits.max_attempts, &mut self.jitter) {
                NextStep::Done(result) => return result,
                NextStep::Retry(seconds) => self.sleeper.sleep(seconds).await,
            }
            attempt = attempt.saturating_add(1);
        }
    }
}
