use std::thread::park_timeout;
use std::time::{Duration, Instant};

use secrecy::SecretString;
use serde::Deserialize;

use crate::credentials::contracts::CliPollData;
use crate::credentials::notice::MessageSink;
use crate::pkce_login::HttpResponse;

use super::RATE_LIMITED_STATUS;
use super::error::PollingError;
use super::transport::PollTransport;
use super::wire;

pub struct PollNotice {
    pub message: String,
    pub every: u32,
}

pub struct PollOptions {
    pub total_timeout_seconds: u64,
    pub poll_interval_seconds: u64,
    pub request_timeout_seconds: u64,
    pub pending: Option<PollNotice>,
    pub other_status: Option<PollNotice>,
    pub http_error_log_every: u32,
    pub connection_error_log_every: u32,
}

pub trait Sleeper {
    fn sleep(&self, interval: Duration);
}

pub struct ThreadSleeper;

impl Sleeper for ThreadSleeper {
    #[cfg_attr(test, mutants::skip)]
    fn sleep(&self, interval: Duration) {
        // std::thread::sleep is a disallowed method; parking this CLI thread is the same wait.
        let started = Instant::now();
        let mut remaining = interval;
        while !remaining.is_zero() {
            park_timeout(remaining);
            remaining = interval.saturating_sub(started.elapsed());
        }
    }
}

enum Attempt {
    Ready(CliPollData),
    Retry,
}

pub struct ReadyPoll<'poll> {
    pub url: &'poll str,
    pub headers: &'poll [(String, SecretString)],
    pub options: &'poll PollOptions,
    pub transport: &'poll dyn PollTransport,
    pub sleeper: &'poll dyn Sleeper,
    pub echo: MessageSink<'poll>,
}

pub fn poll_for_ready_data(poll: &ReadyPoll<'_>) -> Result<Option<CliPollData>, PollingError> {
    let Some(attempts) = poll
        .options
        .total_timeout_seconds
        .checked_div(poll.options.poll_interval_seconds)
    else {
        return Err(PollingError::ZeroPollInterval);
    };
    for attempt in 0..attempts {
        match poll_attempt(poll, attempt)? {
            Attempt::Ready(data) => return Ok(Some(data)),
            Attempt::Retry => poll
                .sleeper
                .sleep(Duration::from_secs(poll.options.poll_interval_seconds)),
        }
    }
    Ok(None)
}

fn poll_attempt(poll: &ReadyPoll<'_>, attempt: u64) -> Result<Attempt, PollingError> {
    let response =
        match poll
            .transport
            .get(poll.url, poll.headers, poll.options.request_timeout_seconds)
        {
            Ok(response) => response,
            Err(source) => {
                if due(poll.options.connection_error_log_every, attempt) {
                    (poll.echo)(&format!("Connection error (will retry): {source}"));
                }
                return Ok(Attempt::Retry);
            }
        };
    if response.status == 200 {
        let data: CliPollData = match wire::decode_object(&response.body) {
            Ok(data) => data,
            Err(source) if source.is_data_error() => {
                return Err(PollingError::PollData { source });
            }
            Err(source) => {
                if due(poll.options.connection_error_log_every, attempt) {
                    (poll.echo)(&format!("Connection error (will retry): {source}"));
                }
                return Ok(Attempt::Retry);
            }
        };
        match data.status.as_deref() {
            Some("ready") => return Ok(Attempt::Ready(data)),
            Some("pending") => log_notice(poll.options.pending.as_ref(), attempt, poll.echo),
            _ => log_notice(poll.options.other_status.as_ref(), attempt, poll.echo),
        }
        return Ok(Attempt::Retry);
    }
    if is_permanent_polling_error(response.status) {
        return Err(rejection(&response, poll.url));
    }
    if due(poll.options.http_error_log_every, attempt) {
        (poll.echo)(&polling_error_message(&response));
    }
    Ok(Attempt::Retry)
}

fn due(every: u32, attempt: u64) -> bool {
    if every == 0 {
        return false;
    }
    attempt.is_multiple_of(u64::from(every))
}

fn log_notice(notice: Option<&PollNotice>, attempt: u64, echo: MessageSink<'_>) {
    let Some(found) = notice else {
        return;
    };
    if due(found.every, attempt) {
        echo(&found.message);
    }
}

fn rejection(response: &HttpResponse, url: &str) -> PollingError {
    response_error_detail(response).map_or_else(
        || PollingError::RejectedWithoutDetail {
            status: response.status,
            url: url.to_owned(),
        },
        |detail| PollingError::Rejected {
            status: response.status,
            detail,
        },
    )
}

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(default)]
    detail: Option<String>,
}

#[must_use]
pub fn response_error_detail(response: &HttpResponse) -> Option<String> {
    let Ok(body) = wire::decode_object::<ErrorBody>(&response.body) else {
        return None;
    };
    body.detail.filter(|detail| !detail.is_empty())
}

#[must_use]
pub fn polling_error_message(response: &HttpResponse) -> String {
    let status = response.status;
    response_error_detail(response).map_or_else(
        || format!("Polling error: HTTP {status}"),
        |detail| format!("Polling error: HTTP {status}: {detail}"),
    )
}

#[must_use]
pub fn is_permanent_polling_error(status: u16) -> bool {
    if status == RATE_LIMITED_STATUS {
        false
    } else {
        (400..500).contains(&status)
    }
}
