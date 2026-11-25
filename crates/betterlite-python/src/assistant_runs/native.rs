use std::sync::Arc;

use betterlite_core::api::assistant_messages::error::MessageFailure;
use betterlite_core::api::assistant_messages::service::{Operation, Reply};
use betterlite_core::api::assistant_run_sdk::error::AssistantRunError;
use betterlite_core::api::assistant_run_sdk::service::RunOutcome;
use betterlite_redact::secret::Redactor;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use serde::Serialize;

use super::convert::{self, MessageCall, RunCall, message_error, run_error};
use super::error::CallError;
use super::session::{ProviderRunStream, Session};
use super::stream::{NativeAssistantStream, deferred};

type Blobs = Vec<Vec<u8>>;
type ReplyObject<'py> = PyResult<Bound<'py, PyAny>>;
type Encoded = PyResult<String>;

// litellm/assistants/main.py's message and run calls: one session per call, the eight functions as methods over it.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeAssistantRunSession {
    session: Arc<Session>,
}

#[cfg_attr(test, mutants::skip)]
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
        let call: MessageCall = convert::read("assistant add message request", text)?;
        let answer = py.detach(|| {
            get_runtime().block_on(self.session.message(&call, Operation::Append, false))
        });
        reply_text(answer, &call.custom_llm_provider, Operation::Append)
    }

    fn a_add_message<'py>(&self, py: Python<'py>, text: &str, _blobs: Blobs) -> ReplyObject<'py> {
        let call: MessageCall = convert::read("assistant add message request", text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, message_work(session, call, Operation::Append))
    }

    fn get_messages(&self, py: Python<'_>, text: &str, _blobs: Blobs) -> Encoded {
        let call: MessageCall = convert::read("assistant get messages request", text)?;
        let answer = py
            .detach(|| get_runtime().block_on(self.session.message(&call, Operation::List, false)));
        reply_text(answer, &call.custom_llm_provider, Operation::List)
    }

    fn aget_messages<'py>(&self, py: Python<'py>, text: &str, _blobs: Blobs) -> ReplyObject<'py> {
        let call: MessageCall = convert::read("assistant get messages request", text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, message_work(session, call, Operation::List))
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
        future_into_py(py, run_work(session, call))
    }

    fn run_stream(
        &self,
        py: Python<'_>,
        text: &str,
        _blobs: Blobs,
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
            deferred(source, provider, Arc::clone(&self.session.redactor)),
        )
    }
}

// The async twins await the same call on the asyncio loop's runtime with the GIL released.
#[cfg_attr(test, mutants::skip)]
async fn message_work(session: Arc<Session>, call: MessageCall, operation: Operation) -> Encoded {
    let provider = call.custom_llm_provider.as_str();
    let answer = session.message(&call, operation, true).await;
    reply_text(answer, provider, operation)
}

#[cfg_attr(test, mutants::skip)]
async fn run_work(session: Arc<Session>, call: RunCall) -> Encoded {
    let provider = call.custom_llm_provider.clone();
    let answer = session.run_sdk().arun_thread(call.into_call()).await;
    run_text(answer, &provider, session.redactor.as_ref())
}

// A message call answers the message add_message built or the page get_messages asked for, never the other.
#[cfg_attr(test, mutants::skip)]
fn reply_text(
    answer: Result<Reply, MessageFailure>,
    provider: &str,
    operation: Operation,
) -> Encoded {
    let reply = answer.map_err(|failure| message_error(&failure, provider).into_exception())?;
    match (reply, operation) {
        (Reply::Message(message), Operation::Append) => encode(&message),
        (Reply::Page(page), Operation::List) => encode(&page),
        (Reply::Page(_), Operation::Append) => Err(PyErr::from(CallError::Reply {
            operation: "add_message",
        })),
        (Reply::Message(_), Operation::List) => Err(PyErr::from(CallError::Reply {
            operation: "get_messages",
        })),
    }
}

#[cfg_attr(test, mutants::skip)]
fn run_text(
    answer: Result<RunOutcome<ProviderRunStream>, AssistantRunError>,
    provider: &str,
    redactor: &Redactor,
) -> Encoded {
    match answer {
        Ok(RunOutcome::Polled(reply)) => {
            let polled = *reply;
            let run = polled.as_ref().ok_or(CallError::Reply {
                operation: "run_thread",
            })?;
            encode(run)
        }
        Ok(RunOutcome::Streaming(_)) => Err(PyErr::from(CallError::Reply {
            operation: "run_thread",
        })),
        Err(error) => Err(run_error(error, provider, redactor).into_exception()),
    }
}

#[cfg_attr(test, mutants::skip)]
fn encode<T>(value: &T) -> Encoded
where
    T: Serialize,
{
    serde_json::to_string(value)
        .map_err(|source| CallError::Encode { source })
        .map_err(PyErr::from)
}
