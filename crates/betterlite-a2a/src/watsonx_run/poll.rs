use std::collections::BTreeMap;
use std::error::Error;
use std::time::Duration;

use thiserror::Error;

use super::event::{RunEvent, RunState};
use super::run_api::{ApiError, Method, call_run};
use super::transport::{RunRequest, RunTransport};

mod tests;

pub const MAX_POLL_ATTEMPTS: u64 = 90;
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Error)]
pub enum PollError<E>
where
    E: Error + 'static,
{
    #[error(transparent)]
    Api(#[from] ApiError<E>),
    #[error("the watsonx Orchestrate run answer has no run_id")]
    NoRunId,
    #[error(
        "the watsonx Orchestrate run '{run_id}' did not reach a terminal state after {seconds}s"
    )]
    PollTimeout { run_id: String, seconds: u64 },
    #[error("the watsonx Orchestrate run ended with a non-success status '{status}'")]
    NonSuccess { status: String },
}

pub async fn poll_run<T>(
    transport: &T,
    base_url: &str,
    run_id: &str,
    headers: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<RunEvent, PollError<T::Error>>
where
    T: RunTransport,
{
    let url = format!("{base_url}/v1/orchestrate/runs/{run_id}");
    for _ in 0..MAX_POLL_ATTEMPTS {
        transport.pause(POLL_INTERVAL).await;
        let request = RunRequest {
            url: &url,
            headers,
            body: None,
            timeout,
        };
        let event = call_run(transport, &request, Method::Get).await?;
        if RunState::parse(event.status.as_deref().unwrap_or_default()).is_some() {
            return Ok(event);
        }
    }
    Err(PollError::PollTimeout {
        run_id: run_id.to_owned(),
        seconds: MAX_POLL_ATTEMPTS.saturating_mul(POLL_INTERVAL.as_secs()),
    })
}

pub async fn successful_run<T>(
    transport: &T,
    event: RunEvent,
    base_url: &str,
    headers: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<RunEvent, PollError<T::Error>>
where
    T: RunTransport,
{
    let status = event.status.as_deref().unwrap_or_default();
    let polled = if RunState::parse(status).is_some() {
        event
    } else {
        let run_id = [event.run_id.as_deref(), event.id.as_deref()]
            .into_iter()
            .flatten()
            .find(|id| !id.is_empty())
            .ok_or(PollError::NoRunId)?
            .to_owned();
        poll_run(transport, base_url, &run_id, headers, timeout).await?
    };
    let final_status = polled.status.as_deref().unwrap_or_default();
    if !matches!(
        RunState::parse(final_status),
        Some(RunState::Completed | RunState::Succeeded)
    ) {
        return Err(PollError::NonSuccess {
            status: final_status.to_owned(),
        });
    }
    Ok(polled)
}
