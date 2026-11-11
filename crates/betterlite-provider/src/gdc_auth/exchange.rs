use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

use crate::gdc_auth::token::TokenReplyError;
use crate::gdc_auth::transport::{
    CaError, GdcClock, GdcJitter, GdcSleeper, TokenExchangeRequest, TokenExchangeResponse,
    TokenTransport, TokenTrust, TransportFailure,
};

pub const MAX_ATTEMPTS: u8 = 3;
pub const FIRST_WAIT_MILLIS: u64 = 1_000;
const RETRYABLE_STATUSES: [u16; 5] = [408, 429, 500, 503, 504];
#[rustfmt::skip]
const RETRYABLE_ERRORS: [&str; 3] = ["internal_failure", "server_error", "temporarily_unavailable"];

#[rustfmt::skip]
pub struct ExchangeRequest { pub url: String, pub body: String, pub trust: TokenTrust }
#[rustfmt::skip]
#[derive(Debug, Error)]
pub enum ExchangeError {
    #[error("the GDC Gemini token endpoint answered with status {status}")] Rejected { status: u16 },
    #[error("the GDC Gemini token request could not be sent")] Transport { #[source] source: TransportFailure },
    #[error("the GDC Gemini token request ran past its deadline")] Deadline,
    #[error("the GDC Gemini token request body could not be written")] Body,
    #[error("the GDC Gemini token reply is not usable")] Reply { #[source] source: TokenReplyError },
    #[error("the GDC Gemini CA certificate could not be loaded")] Ca { #[source] source: CaError },
}
#[rustfmt::skip]
pub struct Exchange<'caller> { pub transport: &'caller dyn TokenTransport, pub sleeper: &'caller dyn GdcSleeper, pub jitter: &'caller dyn GdcJitter, pub clock: &'caller dyn GdcClock }
#[rustfmt::skip]
#[derive(Deserialize)]
struct ErrorReply { #[serde(default)] error: Option<String>, #[serde(default)] error_description: Option<String> }

#[rustfmt::skip]
impl Exchange<'_> {
    pub async fn run(self, request: ExchangeRequest, deadline_millis: u64) -> Result<TokenExchangeResponse, ExchangeError> {
        let deadline = self.clock.monotonic_millis().saturating_add(deadline_millis);
        let mut wait_millis = FIRST_WAIT_MILLIS;
        let mut attempt = 0_u8;
        loop {
            attempt = attempt.saturating_add(1);
            let remaining = deadline.saturating_sub(self.clock.monotonic_millis());
            if remaining == 0 {
                return Err(ExchangeError::Deadline);
            }
            let response = self.transport.exchange(TokenExchangeRequest { url: request.url.clone(), body: request.body.clone(), trust: request.trust.clone(), timeout: Duration::from_millis(remaining) }).await.map_err(|source| ExchangeError::Transport { source })?;
            if response.status == 200 {
                return Ok(response);
            }
            if attempt >= MAX_ATTEMPTS || !retryable(response.status, &response.body) {
                return Err(ExchangeError::Rejected { status: response.status });
            }
            let wait = self.backoff_millis(wait_millis);
            let left = deadline.saturating_sub(self.clock.monotonic_millis());
            self.sleeper.sleep(Duration::from_millis(wait.min(left))).await;
            wait_millis = wait_millis.saturating_mul(2);
        }
    }
    fn backoff_millis(&self, wait_millis: u64) -> u64 {
        let low = wait_millis.saturating_mul(90).saturating_div(100);
        let high = wait_millis.saturating_mul(110).saturating_div(100);
        self.jitter.uniform_millis(low, high)
    }
}

#[rustfmt::skip]
fn retryable(status: u16, body: &[u8]) -> bool {
    if RETRYABLE_STATUSES.contains(&status) {
        return true;
    }
    let Ok(reply) = serde_json::from_slice::<ErrorReply>(body) else {
        return false;
    };
    let error = reply.error.as_deref().unwrap_or_default();
    let description = reply.error_description.as_deref().unwrap_or_default();
    RETRYABLE_ERRORS.contains(&error) || RETRYABLE_ERRORS.contains(&description)
}
