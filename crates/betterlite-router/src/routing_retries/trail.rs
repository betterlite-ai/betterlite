use std::time::Duration;

use crate::cooldown_decisions::failure::FailureClass;

// RETRY_BREADCRUMB_LIMIT is 4: log_retry keeps the three latest failures and appends the new one
const KEPT_BREADCRUMBS: usize = 3;

// One failed attempt as log_retry records it in previous_models.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryAttempt {
    pub model_group: String,
    pub deployment_id: Option<String>,
    pub class: FailureClass,
    pub status: Option<u16>,
    pub attempted_retries: i64,
}

// The retry metadata one request carries for spend logs and the request-wide retry cap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetryTrail {
    pub model_group_size: usize,
    pub attempted_retries: i64,
    pub max_retries: i64,
    pub previous_models: Vec<RetryAttempt>,
    pub request_retry_count: u32,
    pub backoffs: Vec<Duration>,
}

impl RetryTrail {
    // log_retry: every failure counts toward the cap, and only the latest few stay readable
    pub fn record(&mut self, attempt: RetryAttempt) {
        let kept_from = self.previous_models.len().saturating_sub(KEPT_BREADCRUMBS);
        self.previous_models = self.previous_models.split_off(kept_from);
        self.previous_models.push(attempt);
        self.request_retry_count = self.request_retry_count.saturating_add(1);
    }
}
