use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_provider::opensandbox::helpers::{
    OpenSandbox, OpenSandboxError, PollClock, SandboxHttp, SandboxRequest, SandboxStream,
};
use futures_util::StreamExt as _;
use futures_util::stream;

use super::helper_error;
use super::input::{Input, ScriptedResponse};

pub(super) type Environment = BTreeMap<String, String>;
pub(super) type Sandbox = OpenSandbox<RecordingHttp, Environment, ManualClock>;

pub(super) struct RecordingHttp {
    pub(super) responses: Mutex<VecDeque<ScriptedResponse>>,
    pub(super) requests: Mutex<Vec<SandboxRequest>>,
}

pub(super) struct ManualClock {
    pub(super) now: Mutex<Duration>,
}

#[async_trait]
impl SandboxHttp for RecordingHttp {
    async fn send(&self, request: SandboxRequest) -> Result<SandboxStream, OpenSandboxError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let scripted = {
            let mut responses = self
                .responses
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if responses.is_empty() {
                ScriptedResponse {
                    status: 200,
                    body: String::new(),
                }
            } else if responses.len() == 1 {
                responses.front().cloned().unwrap_or_default()
            } else {
                responses.pop_front().unwrap_or_default()
            }
        };
        Ok(SandboxStream {
            status: scripted.status,
            body: stream::iter(vec![Ok(scripted.body.into_bytes())]).boxed(),
        })
    }
}

#[async_trait]
impl PollClock for ManualClock {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn sleep(&self, duration: Duration) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        let next = now.saturating_add(duration);
        *now = next;
    }
}

impl Default for ScriptedResponse {
    fn default() -> Self {
        Self {
            status: 200,
            body: String::new(),
        }
    }
}

pub(super) fn recording_sandbox(input: &Input) -> Sandbox {
    OpenSandbox {
        http: RecordingHttp {
            responses: Mutex::new(input.scripted()),
            requests: Mutex::new(Vec::new()),
        },
        environment: Environment::new(),
        clock: ManualClock {
            now: Mutex::new(Duration::ZERO),
        },
    }
}

pub(super) fn block_on<Output, Work>(work: Work) -> Result<Output, Box<crate::helpers::HelperError>>
where
    Work: Future<Output = Result<Output, OpenSandboxError>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error("RuntimeError", error))?;
    runtime
        .block_on(work)
        .map_err(|error| helper_error("OpenSandboxError", error))
}
