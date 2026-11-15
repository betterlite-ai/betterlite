use std::future::Future;
use std::str;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use tokio::time::Instant;
use url::Url;

use crate::api::bfl::common::{
    BflError, BflReply, DEFAULT_MAX_POLLING_TIME, DEFAULT_POLLING_INTERVAL, check_bfl_poll_url,
    decode_bfl_reply,
};

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum PollError {
    #[error("the BFL submission answered HTTP {status}: {body}")]
    InitialStatus { status: u16, body: String },
    #[error("the BFL submission answer is not a task: {source}")]
    InitialDecode {
        status: u16,
        #[source]
        source: BflError,
    },
    #[error("the BFL submission answer reports an error: {body}")]
    InitialErrors { status: u16, body: String },
    #[error("No polling_url in BFL response")]
    MissingPollingUrl,
    #[error("the BFL polling URL is not trusted: {source}")]
    PollingUrl {
        #[source]
        source: BflError,
    },
    #[error("the BFL poll answered HTTP {status}: {body}")]
    PollStatus { status: u16, body: String },
    #[error("the BFL poll answer is not a task: {source}")]
    PollDecode {
        #[source]
        source: BflError,
    },
    #[error("the BFL poll request failed")]
    PollTransport {
        #[source]
        source: TransportError,
    },
    #[error("Image generation failed: {state}")]
    Failed { state: String },
    #[error("the BFL generation poll ran past its 300-second limit")]
    TimedOut,
}

impl PollError {
    // LiteLLM synthesizes the initial status for both a rejected and an unparseable submission, and the poll status for a rejected poll
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::InitialStatus { status }
            | Self::InitialDecode { status, .. }
            | Self::InitialErrors { status, .. }
            | Self::PollStatus { status, .. } => Some(*status),
            Self::MissingPollingUrl => Some(500),
            Self::PollingUrl { source } => source.status(),
            Self::Failed { .. } => Some(400),
            Self::TimedOut => Some(408),
            Self::PollDecode { .. } | Self::PollTransport { .. } => None,
        }
    }
}

pub trait PollTransport {
    fn get(
        &self,
        url: Url,
        headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

// LiteLLM's poll passes no timeout, so the provider client's own read limit bounds each GET
impl PollTransport for ProxiedClient {
    async fn get(&self, url: Url, headers: HeaderMap) -> Result<UpstreamResponse, TransportError> {
        self.send_buffered(OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Empty,
            deadline: self.limits.read_timeout,
        })
        .await
    }
}

pub trait PollClock {
    fn now(&self) -> Duration;
}

pub trait PollSleeper {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}

#[derive(Debug, Clone, Copy)]
pub struct MonotonicClock {
    pub origin: Instant,
}

impl PollClock for MonotonicClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TokioSleeper;

impl PollSleeper for TokioSleeper {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

pub struct BflPoller<'client, T, C, S> {
    pub transport: &'client T,
    pub clock: &'client C,
    pub sleeper: &'client S,
}

impl<T, C, S> BflPoller<'_, T, C, S>
where
    T: PollTransport + Sync,
    C: PollClock + Sync,
    S: PollSleeper + Sync,
{
    // LiteLLM's sync and async poll bodies differ only in the client and the sleep call, so one state machine serves both
    pub async fn poll(
        &self,
        initial: &UpstreamResponse,
        headers: &HeaderMap,
    ) -> Result<BflReply, PollError> {
        let reply = initial_reply(initial)?;
        let polling_url = reply
            .polling_url
            .as_deref()
            .filter(|url| !url.is_empty())
            .ok_or(PollError::MissingPollingUrl)?;
        // the credential leaves the client only for a host B1517 trusts
        let url =
            check_bfl_poll_url(polling_url).map_err(|source| PollError::PollingUrl { source })?;
        let polling_headers = polling_headers(headers);
        let started = self.clock.now();
        loop {
            if self.clock.now().saturating_sub(started) >= DEFAULT_MAX_POLLING_TIME {
                return Err(PollError::TimedOut);
            }
            let response = self
                .transport
                .get(url.clone(), polling_headers.clone())
                .await
                .map_err(|source| PollError::PollTransport { source })?;
            if response.status != 200 {
                return Err(PollError::PollStatus {
                    status: response.status,
                    body: response_text(&response.body),
                });
            }
            let reply = decode_bfl_reply(&response.body)
                .map_err(|source| PollError::PollDecode { source })?;
            match reply.status.as_deref() {
                Some("Ready") => return Ok(reply),
                Some(state @ ("Error" | "Failed" | "Content Moderated" | "Request Moderated")) => {
                    return Err(PollError::Failed {
                        state: state.to_owned(),
                    });
                }
                _ => self.sleeper.sleep(DEFAULT_POLLING_INTERVAL).await,
            }
        }
    }
}

// the status check comes first, so a rejection keeps its own status even when its body is not JSON
fn initial_reply(initial: &UpstreamResponse) -> Result<BflReply, PollError> {
    if initial.status >= 400 {
        return Err(PollError::InitialStatus {
            status: initial.status,
            body: response_text(&initial.body),
        });
    }
    let reply = decode_bfl_reply(&initial.body).map_err(|source| PollError::InitialDecode {
        status: initial.status,
        source,
    })?;
    // the key alone rejects the submission, even when its value is null or an empty list
    if reply.errors.is_some() {
        return Err(PollError::InitialErrors {
            status: initial.status,
            body: response_text(&initial.body),
        });
    }
    Ok(reply)
}

// LiteLLM forwards just x-key, defaulting it to an empty string when the submission had none
fn polling_headers(headers: &HeaderMap) -> HeaderMap {
    let mut polling = HeaderMap::new();
    let key = headers
        .get("x-key")
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static(""));
    polling.insert(HeaderName::from_static("x-key"), key);
    polling
}

fn response_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", body.len()),
        str::to_owned,
    )
}
