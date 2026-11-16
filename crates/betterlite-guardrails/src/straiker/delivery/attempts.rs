use indexmap::IndexMap;
use serde::Serialize;

use crate::straiker::delivery::block::record_verdict;
use crate::straiker::delivery::headers::{self, WebhookHeaders};
use crate::straiker::delivery::payload;
use crate::straiker::delivery::responses::Verdict;
use crate::straiker::delivery::retry::{RetryDelay, backoff_cap};
use crate::straiker::delivery::transport::{WebhookPost, WebhookReply, WebhookRequest};
use crate::straiker::delivery::{ApplyRequest, StraikerDelivery};
use crate::straiker_webhook::failure::{WebhookFailure, status_failure};

impl<Post, Delay> StraikerDelivery<Post, Delay>
where
    Post: WebhookPost,
    Delay: RetryDelay,
{
    pub(super) async fn post<Value>(
        &self,
        payload: &Value,
        extra: &IndexMap<&'static str, String>,
    ) -> Result<WebhookReply, WebhookFailure>
    where
        Value: Serialize,
    {
        let body = payload::encode(payload, self.settings.max_payload_bytes).map_err(|error| {
            WebhookFailure {
                message: error.to_string(),
                is_unreachable: false,
                retryable: false,
            }
        })?;
        let url = self.settings.webhook_url();
        self.log_request(&url, &body);
        let merged = headers::compose(&self.settings, extra);
        let attempts = self.settings.max_retries.saturating_add(1);
        let mut attempt = 0_i64;
        loop {
            match self.deliver(&url, &body, &merged).await {
                Ok(reply) => return Ok(reply),
                Err(failure) => {
                    if !failure.retryable || attempt.saturating_add(1) >= attempts {
                        return Err(failure);
                    }
                    let cap = backoff_cap(
                        self.settings.initial_backoff,
                        self.settings.max_backoff,
                        attempt,
                    );
                    tokio::time::sleep(self.delay.delay(cap)).await;
                    attempt = attempt.saturating_add(1);
                }
            }
        }
    }

    async fn deliver(
        &self,
        url: &str,
        body: &str,
        headers: &WebhookHeaders,
    ) -> Result<WebhookReply, WebhookFailure> {
        let request = WebhookRequest {
            url,
            body,
            headers,
            timeout: self.settings.timeout,
        };
        let reply = self.transport.post(request).await?;
        if reply.status == 200 {
            return Ok(reply);
        }
        Err(status_failure(reply.status, &reply.body))
    }

    pub(super) fn record(&self, request: &mut ApplyRequest<'_>, verdict: &Verdict) {
        if !self.settings.verbose {
            return;
        }
        record_verdict(
            request.request_data,
            verdict.action.wire_name(),
            verdict.turn_id.as_deref(),
        );
    }

    fn log_request(&self, url: &str, body: &str) {
        if !self.settings.verbose {
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

    pub(super) fn log_response(&self, reply: &WebhookReply) {
        if !self.settings.verbose {
            return;
        }
        tracing::info!(
            event = "straiker.webhook_response",
            status_code = reply.status,
            body = %reply.body,
            "straiker.webhook_response"
        );
    }

    pub(super) fn log_replay(&self, scope: &str, prefix: &str) {
        if !self.settings.verbose {
            return;
        }
        tracing::info!(
            event = "straiker.replay_blocked",
            scope = %scope,
            prefix = %prefix,
            "straiker.replay_blocked"
        );
    }
}
