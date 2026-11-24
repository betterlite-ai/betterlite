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

impl NativeAssistantStream {
    pub(crate) fn deferred(
        source: ProviderRunStream,
        provider: String,
        redactor: Arc<Redactor>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(StreamState {
                source: Some(source),
                events: None,
                closed: false,
            })),
            provider,
            redactor,
        }
    }
}

#[pymethods]
impl NativeAssistantStream {
    // the blocking form, running the reads with the GIL released
    fn next_event(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let state = Arc::clone(&self.state);
        let provider = self.provider.clone();
        let redactor = Arc::clone(&self.redactor);
        py.detach(move || get_runtime().block_on(advance(&state, &provider, redactor.as_ref())))
    }

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

    fn aclose<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.close(py);
        future_into_py(py, async { Ok::<(), PyErr>(()) })
    }
}

// What one advance does next: read the opened events, or open the manager core built.
enum Step {
    Read(RunEventStream),
    Open(ProviderRunStream),
}

// The stream is taken out of the lock while it waits, so close() never waits on a read.
async fn advance(
    state: &Mutex<StreamState>,
    provider: &str,
    redactor: &Redactor,
) -> PyResult<Option<String>> {
    let step = {
        let mut guard = state.lock().await;
        if guard.closed {
            return Ok(None);
        }
        if let Some(events) = guard.events.take() {
            Step::Read(events)
        } else if let Some(source) = guard.source.take() {
            Step::Open(source)
        } else {
            guard.closed = true;
            return Ok(None);
        }
    };
    let mut events = match step {
        Step::Read(events) => events,
        Step::Open(source) => match source.events().await {
            Ok(opened) => opened,
            Err(error) => {
                state.lock().await.closed = true;
                return Err(stream_exception(error, provider, redactor));
            }
        },
    };
    let next = events.next().await;
    let mut guard = state.lock().await;
    if guard.closed {
        return Ok(None);
    }
    match next {
        None => {
            guard.closed = true;
            Ok(None)
        }
        Some(Ok(event)) => {
            guard.events = Some(events);
            encode_event(&event).map(Some)
        }
        Some(Err(error)) => {
            guard.closed = true;
            Err(stream_exception(error, provider, redactor))
        }
    }
}

// A stream failure maps the way arun_thread maps one, since the SDK's own exception class is not in the wheel.
fn stream_exception(error: ThreadRunsError, provider: &str, redactor: &Redactor) -> PyErr {
    let dispatch = AssistantRunError::Dispatch(AssistantsRunError::Runs(error));
    run_error(dispatch, provider, redactor).into_exception()
}

fn encode_event(event: &RunEvent) -> PyResult<String> {
    serde_json::to_string(event).map_err(|source| PyErr::from(CallError::Encode { source }))
}
