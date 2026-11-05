use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_http::url_path::encode_segment;
use http::{HeaderMap, Method};
use tokio::time::{Instant, sleep};
use url::Url;

use crate::api::runway::tasks::TaskKind;
use crate::api::runway::tasks::error::RunwayTaskError;
use crate::api::runway::tasks::record::{RunwayTask, TaskState, decode_runway_task};

const POLL_INTERVAL: Duration = Duration::from_secs(2);

pub trait TaskTransport {
    fn get(
        &self,
        url: Url,
        headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl TaskTransport for ProxiedClient {
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
    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}

#[derive(Debug, Clone, Copy)]
pub struct MonotonicClock {
    pub origin: Instant,
}

impl PollClock for MonotonicClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    async fn pause(&self, duration: Duration) {
        sleep(duration).await;
    }
}

pub struct TaskPoller<'client, T, C> {
    pub transport: &'client T,
    pub clock: &'client C,
    pub kind: TaskKind,
    pub timeout_seconds: u64,
}

impl<T: TaskTransport + Sync, C: PollClock + Sync> TaskPoller<'_, T, C> {
    pub async fn poll(
        &self,
        api_base: &str,
        task_id: &str,
        headers: &HeaderMap,
    ) -> Result<RunwayTask, RunwayTaskError> {
        let segment = encode_segment(task_id, "task_id").map_err(RunwayTaskError::TaskId)?;
        let task_url = Url::parse(&format!(
            "{}/v1/tasks/{segment}",
            api_base.trim_end_matches('/')
        ))
        .map_err(RunwayTaskError::TaskUrl)?;
        let started = self.clock.now();
        let deadline = Duration::from_secs(self.timeout_seconds);
        loop {
            if self.clock.now().saturating_sub(started) > deadline {
                return Err(RunwayTaskError::TimedOut {
                    kind: self.kind,
                    seconds: self.timeout_seconds,
                });
            }
            let response = self
                .transport
                .get(task_url.clone(), headers.clone())
                .await
                .map_err(RunwayTaskError::Transport)?;
            if !(200..300).contains(&response.status) {
                return Err(RunwayTaskError::HttpStatus {
                    status: response.status,
                });
            }
            let task = decode_runway_task(&response.body)?;
            match task.state() {
                TaskState::Succeeded => return Ok(task),
                TaskState::Running => self.clock.pause(POLL_INTERVAL).await,
                TaskState::Failed => return Err(task.failure_error(self.kind)),
                TaskState::Cancelled => return Err(RunwayTaskError::Cancelled { kind: self.kind }),
                TaskState::Unknown(status) => {
                    return Err(RunwayTaskError::UnknownStatus { status });
                }
            }
        }
    }
}
