use std::sync::Arc;

use betterlite_azure::assistants::error::AssistantsRunError;
use betterlite_core::api::assistant_run_sdk::error::AssistantRunError;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::records::RunEvent;
use betterlite_openai::thread_runs::stream::RunEventStream;
use betterlite_redact::secret::Redactor;
use futures_util::StreamExt as _;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use tokio::sync::Mutex;

use super::convert::run_error;
use super::error::CallError;
use super::session::ProviderRunStream;

mod tests;

// The lazy manager core built waits here until the first event read, which is when the provider is called.
struct StreamState {
    source: Option<ProviderRunStream>,
    events: Option<RunEventStream>,
    closed: bool,
}

// The event iterator a streaming run returns; each event arrives as JSON text, and None is the end.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeAssistantStream {
    state: Arc<Mutex<StreamState>>,
    provider: String,
    redactor: Arc<Redactor>,
}

pub(crate) fn deferred(
    source: ProviderRunStream,
    provider: String,
    redactor: Arc<Redactor>,
) -> NativeAssistantStream {
    NativeAssistantStream {
        state: Arc::new(Mutex::new(StreamState {
            source: Some(source),
            events: None,
            closed: false,
        })),
        provider,
        redactor,
    }
}

#[pymethods]
impl NativeAssistantStream {
    #[cfg_attr(test, mutants::skip)]
    fn next_event(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let state = Arc::clone(&self.state);
        let provider = self.provider.clone();
        let redactor = Arc::clone(&self.redactor);
        py.detach(move || get_runtime().block_on(advance(&state, &provider, redactor.as_ref())))
    }

    #[cfg_attr(test, mutants::skip)]
    fn anext_event<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let state = Arc::clone(&self.state);
        let provider = self.provider.clone();
        let redactor = Arc::clone(&self.redactor);
        future_into_py(py, async move {
            advance(&state, &provider, redactor.as_ref()).await
        })
    }

    fn close(&self, py: Python<'_>) {
        let state = Arc::clone(&self.state);
        py.detach(move || {
            get_runtime().block_on(async move {
                let mut guard = state.lock().await;
                guard.closed = true;
                guard.source = None;
                guard.events = None;
            });
        });
    }

    #[cfg_attr(test, mutants::skip)]
    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.close(py);
        future_into_py(py, async { Ok::<(), PyErr>(()) })
    }
}

enum Step {
    Read(RunEventStream),
    Open(Box<ProviderRunStream>),
}

#[cfg_attr(test, mutants::skip)]
async fn pick_step(state: &Mutex<StreamState>) -> Option<Step> {
    let mut guard = state.lock().await;
    if guard.closed {
        return None;
    }
    if let Some(events) = guard.events.take() {
        return Some(Step::Read(events));
    }
    if let Some(source) = guard.source.take() {
        return Some(Step::Open(Box::new(source)));
    }
    guard.closed = true;
    None
}

#[cfg_attr(test, mutants::skip)]
async fn read_step(
    step: Step,
) -> (
    Option<RunEventStream>,
    Option<Result<RunEvent, ThreadRunsError>>,
) {
    match step {
        Step::Read(mut events) => {
            let next = events.next().await;
            (Some(events), next)
        }
        Step::Open(source) => match source.events().await {
            Ok(mut events) => {
                let next = events.next().await;
                (Some(events), next)
            }
            Err(error) => (None, Some(Err(error))),
        },
    }
}

#[cfg_attr(test, mutants::skip)]
async fn advance(
    state: &Mutex<StreamState>,
    provider: &str,
    redactor: &Redactor,
) -> PyResult<Option<String>> {
    let Some(step) = pick_step(state).await else {
        return Ok(None);
    };
    let (events, next) = read_step(step).await;
    let mut guard = state.lock().await;
    if guard.closed {
        return Ok(None);
    }
    match next {
        None => {
            guard.closed = true;
            drop(guard);
            Ok(None)
        }
        Some(Ok(event)) => {
            guard.events = events;
            drop(guard);
            serde_json::to_string(&event)
                .map_err(|source| PyErr::from(CallError::Encode { source }))
                .map(Some)
        }
        Some(Err(error)) => {
            guard.closed = true;
            drop(guard);
            let dispatch = AssistantRunError::Dispatch(AssistantsRunError::Runs(error));
            Err(run_error(dispatch, provider, redactor).into_exception())
        }
    }
}
