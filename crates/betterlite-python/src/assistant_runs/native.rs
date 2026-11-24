use std::sync::Arc;

use betterlite_core::api::assistant_messages::error::MessageFailure;
use betterlite_core::api::assistant_messages::service::Reply;
use betterlite_core::api::assistant_run_sdk::error::AssistantRunError;
use betterlite_core::api::assistant_run_sdk::service::RunOutcome;
use betterlite_redact::secret::Redactor;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use serde::Serialize;

use super::convert::{self, AppendCall, ListCall, RunCall, message_error, run_error};
use super::error::CallError;
use super::session::{ProviderRunStream, Session};
use super::stream::NativeAssistantStream;

// The attachment bytes Python passes with every call; the assistant calls never read them.
type Blobs = Vec<Vec<u8>>;

// What a coroutine-returning method hands back to Python.
type ReplyObject<'py> = PyResult<Bound<'py, PyAny>>;

// The JSON text a function returns, or the exception Python raises in its place.
type Encoded = PyResult<String>;

// litellm/assistants/main.py's message and run calls: one session per call, the eight functions as methods over it.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeAssistantRunSession {
    session: Arc<Session>,
}

impl NativeAssistantRunSession {
    // Both stream methods defer the provider call to the first event read, as the SDK's lazy managers do.
    fn opened_stream(
        &self,
        py: Python<'_>,
        text: &str,
        asynchronous: bool,
    ) -> PyResult<Py<NativeAssistantStream>> {
        let call: RunCall = convert::read("assistant run request", text)?;
        let provider = call.custom_llm_provider.clone();
        let sdk = self.session.run_sdk();
        let opened = if asynchronous {
            sdk.arun_thread_stream(call.into_call())
        } else {
            sdk.run_thread_stream(call.into_call())
        };
        let source = opened.map_err(|error| {
            run_error(error, &provider, self.session.redactor.as_ref()).into_exception()
        })?;
        Py::new(
            py,
            NativeAssistantStream::deferred(source, provider, Arc::clone(&self.session.redactor)),
        )
    }
}

#[pymethods]
impl NativeAssistantRunSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let session = Session::build(settings)?;
        Ok(Self {
            session: Arc::new(session),
        })
    }

    fn add_message(&self, py: Python<'_>, text: &str, _blobs: Blobs) -> Encoded {
        let call: AppendCall = convert::read("assistant add message request", text)?;
        let answer = py.detach(|| get_runtime().block_on(self.session.append(&call, false)));
        message_text(answer, &call.custom_llm_provider)
    }

    fn a_add_message<'py>(&self, py: Python<'py>, text: &str, _blobs: Blobs) -> ReplyObject<'py> {
        let call: AppendCall = convert::read("assistant add message request", text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move {
            let answer = session.append(&call, true).await;
            message_text(answer, &call.custom_llm_provider)
        })
    }

    fn get_messages(&self, py: Python<'_>, text: &str, _blobs: Blobs) -> Encoded {
        let call: ListCall = convert::read("assistant get messages request", text)?;
        let answer = py.detach(|| get_runtime().block_on(self.session.list(&call, false)));
        page_text(answer, &call.custom_llm_provider)
    }

    fn aget_messages<'py>(&self, py: Python<'py>, text: &str, _blobs: Blobs) -> ReplyObject<'py> {
        let call: ListCall = convert::read("assistant get messages request", text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move {
            let answer = session.list(&call, true).await;
            page_text(answer, &call.custom_llm_provider)
        })
    }

    fn run_thread(&self, py: Python<'_>, text: &str, _blobs: Blobs) -> Encoded {
        let call: RunCall = convert::read("assistant run request", text)?;
        let provider = call.custom_llm_provider.clone();
        let answer = py
            .detach(|| get_runtime().block_on(self.session.run_sdk().run_thread(call.into_call())));
        run_text(answer, &provider, self.session.redactor.as_ref())
    }

    fn arun_thread<'py>(&self, py: Python<'py>, text: &str, _blobs: Blobs) -> ReplyObject<'py> {
        let call: RunCall = convert::read("assistant run request", text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move {
            let provider = call.custom_llm_provider.clone();
            let answer = session.run_sdk().arun_thread(call.into_call()).await;
            run_text(answer, &provider, session.redactor.as_ref())
        })
    }

    fn run_thread_stream(
        &self,
        py: Python<'_>,
        text: &str,
        _blobs: Blobs,
    ) -> PyResult<Py<NativeAssistantStream>> {
        self.opened_stream(py, text, false)
    }

    fn arun_thread_stream(
        &self,
        py: Python<'_>,
        text: &str,
        _blobs: Blobs,
    ) -> PyResult<Py<NativeAssistantStream>> {
        self.opened_stream(py, text, true)
    }
}

// add_message returns the message and get_messages the page; a reply of the other kind never leaves Rust.
fn message_text(answer: Result<Reply, MessageFailure>, provider: &str) -> Encoded {
    let reply = answer.map_err(|failure| message_error(failure, provider).into_exception())?;
    match reply {
        Reply::Message(message) => encode(&message),
        Reply::Page(_) => Err(PyErr::from(CallError::Reply {
            operation: "add_message",
        })),
    }
}

fn page_text(answer: Result<Reply, MessageFailure>, provider: &str) -> Encoded {
    let reply = answer.map_err(|failure| message_error(failure, provider).into_exception())?;
    match reply {
        Reply::Page(page) => encode(&page),
        Reply::Message(_) => Err(PyErr::from(CallError::Reply {
            operation: "get_messages",
        })),
    }
}

fn run_text(
    answer: Result<RunOutcome<ProviderRunStream>, AssistantRunError>,
    provider: &str,
    redactor: &Redactor,
) -> Encoded {
    match answer {
        Ok(RunOutcome::Polled(reply)) => match *reply {
            Some(run) => encode(&run),
            None => Err(PyErr::from(CallError::Reply {
                operation: "run_thread",
            })),
        },
        Ok(RunOutcome::Streaming(_)) => Err(PyErr::from(CallError::Reply {
            operation: "run_thread",
        })),
        Err(error) => Err(run_error(error, provider, redactor).into_exception()),
    }
}

fn encode<T>(value: &T) -> Encoded
where
    T: Serialize,
{
    serde_json::to_string(value)
        .map_err(|source| CallError::Encode { source })
        .map_err(PyErr::from)
}
