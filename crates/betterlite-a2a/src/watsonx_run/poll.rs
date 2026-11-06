use std::collections::BTreeMap;
use std::error::Error;
use std::time::Duration;

use thiserror::Error;

use super::event::{RunEvent, RunState};
use super::run_api::{ApiError, Method, call_run};
use super::transport::{RunLimits, RunRequest, RunTransport};

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

#[must_use]
pub fn run_id(event: &RunEvent) -> Option<&str> {
    [event.run_id.as_deref(), event.id.as_deref()]
        .into_iter()
        .flatten()
        .find(|id| !id.is_empty())
}

pub async fn poll_run<T>(
    transport: &T,
    runs_url: &str,
    started: RunEvent,
    headers: &BTreeMap<String, String>,
    limits: RunLimits,
) -> Result<RunEvent, PollError<T::Error>>
where
    T: RunTransport,
{
    let mut run = started;
    if RunState::parse(run.status.as_deref().unwrap_or_default()).is_none() {
        let id = run_id(&run).ok_or(PollError::NoRunId)?.to_owned();
        let url = format!("{runs_url}/{id}");
        let mut polled = None;
        for _ in 0..MAX_POLL_ATTEMPTS {
            transport.pause(POLL_INTERVAL).await;
            let request = RunRequest {
                url: &url,
                headers,
                body: None,
                limits,
            };
            let event = call_run(transport, &request, Method::Get).await?;
            if RunState::parse(event.status.as_deref().unwrap_or_default()).is_some() {
                polled = Some(event);
                break;
            }
        }
        run = polled.ok_or_else(|| PollError::PollTimeout {
            run_id: id,
            seconds: MAX_POLL_ATTEMPTS.saturating_mul(POLL_INTERVAL.as_secs()),
        })?;
    }
    match RunState::parse(run.status.as_deref().unwrap_or_default()) {
        Some(RunState::Completed | RunState::Succeeded) => Ok(run),
        _ => Err(PollError::NonSuccess {
            status: run.status.unwrap_or_default(),
        }),
    }
}
