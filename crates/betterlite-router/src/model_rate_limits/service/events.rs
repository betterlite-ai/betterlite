use betterlite_tokenizer::reservation::RequestBody;

use crate::io_token_reservations::clock::minute_label;

use super::super::error::RATE_LIMIT_TTL_SECONDS;
use super::{ModelRateLimits, SuccessUsage};

// ModelRateLimitingCheck.async_log_success_event and its blocking twin: a response that reported both a model name and tokens counts its tpm usage
impl ModelRateLimits {
    pub async fn async_log_success_event(&self, usage: &SuccessUsage<'_>) {
        if let Some(key) = self.success_key(usage) {
            let _counted = self
                .cache
                .increment(&key, usage.total_tokens, RATE_LIMIT_TTL_SECONDS)
                .await;
        }
    }

    // log_success_event: the blocking twin counts the in-memory tier only
    pub fn log_success_event(&self, usage: &SuccessUsage<'_>) {
        if let Some(key) = self.success_key(usage) {
            let _counted =
                self.cache
                    .increment_local(&key, usage.total_tokens, RATE_LIMIT_TTL_SECONDS);
        }
    }

    // async_log_failure_event: never fail the primary logging pipeline over an io-token refund error
    pub async fn async_log_failure_event(&self, request: &mut RequestBody) {
        if let Some(hook) = &self.io {
            hook.async_refund(request).await;
        }
    }

    // log_failure_event: the blocking twin
    pub fn log_failure_event(&self, request: &mut RequestBody) {
        if let Some(hook) = &self.io {
            hook.refund(request);
        }
    }

    fn success_key(&self, usage: &SuccessUsage<'_>) -> Option<String> {
        let model_id = usage.model_id?;
        let model = usage.model.filter(|name| !name.is_empty())?;
        if usage.total_tokens == 0 {
            return None;
        }
        Some(format!(
            "{model_id}:{model}:tpm:{}",
            minute_label((self.clock)())
        ))
    }
}
