use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use indexmap::IndexMap;
use rand::RngExt as _;
use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysError, SysRng};
use serde::Serialize;
use thiserror::Error;
use tokio::time::sleep;

use crate::straiker::delivery::StraikerDelivery;
use crate::straiker::delivery::settings::StraikerSettings;
use crate::straiker::delivery::transport::{self, WebhookPost, WebhookReply, WebhookRequest};
use crate::straiker_webhook::failure::{WebhookFailure, status_failure};

#[derive(Debug, Error)]
pub enum PayloadError {
    #[error("request serialization failed: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("payload {bytes}B exceeds max_payload_bytes {limit}")]
    TooLarge { bytes: usize, limit: usize },
}

pub fn encode<Value>(value: &Value, max_payload_bytes: usize) -> Result<String, PayloadError>
where
    Value: Serialize,
{
    let body = serde_json::to_string(value).map_err(PayloadError::Serialize)?;
    if body.len() > max_payload_bytes {
        return Err(PayloadError::TooLarge {
            bytes: body.len(),
            limit: max_payload_bytes,
        });
    }
    Ok(body)
}

pub trait RetryDelay: Sync {
    fn delay(&self, cap: Duration) -> Duration;
}

pub struct RandomDelay {
    random: Mutex<StdRng>,
}

#[derive(Debug, Error)]
pub enum RandomError {
    #[error("the retry jitter source could not be seeded")]
    Entropy(#[from] SysError),
}

impl RandomDelay {
    pub fn try_new() -> Result<Self, RandomError> {
        Ok(Self {
            random: Mutex::new(StdRng::try_from_rng(&mut SysRng)?),
        })
    }
}

impl RetryDelay for RandomDelay {
    // LiteLLM sleeps random.uniform(0, backoff): a draw from zero to the exponential cap.
    fn delay(&self, cap: Duration) -> Duration {
        let upper = cap.as_secs_f64();
        let seconds = self
            .random
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .random::<f64>()
            .mul_add(upper, 0.0_f64);
        Duration::try_from_secs_f64(seconds).unwrap_or_default()
    }
}

// min(initial_backoff * 2**attempt, max_backoff) in seconds; attempt 0 is the wait after the first failure.
#[must_use]
pub fn backoff_cap(initial_backoff: f64, max_backoff: f64, attempt: i64) -> Duration {
    let exponent = i32::try_from(attempt).unwrap_or(i32::MAX);
    let step = 2.0_f64.powi(exponent);
    let seconds = initial_backoff.mul_add(step, 0.0_f64).min(max_backoff);
    Duration::try_from_secs_f64(seconds).unwrap_or_default()
}

pub(super) async fn post<Post, Delay>(
    delivery: &StraikerDelivery<Post, Delay>,
    body: &str,
    extra: &IndexMap<&'static str, String>,
) -> Result<WebhookReply, WebhookFailure>
where
    Post: WebhookPost,
    Delay: RetryDelay,
{
    let settings = &delivery.settings;
    let url = settings.webhook_url();
    log_request(settings, &url, body);
    let headers = transport::compose(settings, extra);
    let attempts = settings.max_retries.saturating_add(1);
    let mut attempt = 0_i64;
    loop {
        let request = WebhookRequest {
            url: &url,
            body,
            headers: &headers,
            timeout: settings.timeout,
        };
        let failure = match delivery.transport.post(request).await {
            Ok(reply) if reply.status == 200 => return Ok(reply),
            Ok(reply) => status_failure(reply.status, &reply.body),
            Err(failure) => failure,
        };
        if !failure.retryable || attempt.saturating_add(1) >= attempts {
            return Err(failure);
        }
        let cap = backoff_cap(settings.initial_backoff, settings.max_backoff, attempt);
        sleep(delivery.delay.delay(cap)).await;
        attempt = attempt.saturating_add(1);
    }
}

fn log_request(settings: &StraikerSettings, url: &str, body: &str) {
    if !settings.verbose {
        return;
    }
    tracing::info!(
        event = "straiker.webhook_request",
        url = %url,
        bytes = body.len(),
        payload = %body,
        "straiker.webhook_request"
    );
}

pub(super) fn log_response(settings: &StraikerSettings, reply: &WebhookReply) {
    if !settings.verbose {
        return;
    }
    tracing::info!(
        event = "straiker.webhook_response",
        status_code = reply.status,
        body = %reply.body,
        "straiker.webhook_response"
    );
}

pub(super) fn log_replay(settings: &StraikerSettings, scope: &str, prefix: &str) {
    if !settings.verbose {
        return;
    }
    tracing::info!(
        event = "straiker.replay_blocked",
        scope = %scope,
        prefix = %prefix,
        "straiker.replay_blocked"
    );
}
