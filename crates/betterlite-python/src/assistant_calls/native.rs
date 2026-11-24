use std::sync::Arc;

use betterlite_core::api::assistant_read_delete::AssistantStep;
use betterlite_core::api::assistant_read_delete::request::AssistantReadDeleteInput;
use betterlite_core::api::assistant_records::AssistantCreateInput;
use betterlite_core::api::assistant_records::error::AssistantCreateFailure;
use betterlite_openai::assistants::records::AssistantObject;
use betterlite_openai::threads::records::ThreadRecord;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};
use serde::Serialize;

use super::convert::{self, ThreadRequest};
use super::error::family::ThreadOutcome;
use super::error::{CallError, family};
use super::session::{Session, ThreadKind};

// The attachment bytes Python passes with every call; the assistant endpoints never read them.
type Blobs = Vec<Vec<u8>>;

// What a coroutine-returning method hands back to Python.
type Reply<'py> = PyResult<Bound<'py, PyAny>>;

// The JSON text a function returns, or the exception Python raises in its place.
type Encoded = PyResult<String>;

// Which assistant object a read or delete call answers with, so one reply builds either.
enum Answer {
    Page,
    Deleted,
}

// litellm/assistants/main.py: one session per call, the ten functions as methods over it.
#[pyclass(module = "betterlite._native")]
pub(crate) struct NativeAssistantSession {
    session: Arc<Session>,
}

#[pymethods]
impl NativeAssistantSession {
    #[new]
    fn new(settings: &str) -> PyResult<Self> {
        let session = Session::build(settings)?;
        Ok(Self {
            session: Arc::new(session),
        })
    }

    fn get_assistants(&self, py: Python<'_>, text: &str, _blobs: Blobs) -> Encoded {
        let input = convert::read("assistant read or delete request", text)?;
        let step = read_delete_step(py, &self.session, &input)?;
        reply_text(step, Answer::Page)
    }

    fn aget_assistants<'py>(&self, py: Python<'py>, text: &str, _blobs: Blobs) -> Reply<'py> {
        let input = convert::read("assistant read or delete request", text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, read_delete_work(session, input, Answer::Page))
    }

    fn delete_assistant(&self, py: Python<'_>, text: &str, _blobs: Blobs) -> Encoded {
        let input = convert::read("assistant read or delete request", text)?;
        let step = read_delete_step(py, &self.session, &input)?;
        reply_text(step, Answer::Deleted)
    }

    fn adelete_assistant<'py>(&self, py: Python<'py>, text: &str, _blobs: Blobs) -> Reply<'py> {
        let input = convert::read("assistant read or delete request", text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, read_delete_work(session, input, Answer::Deleted))
    }

    fn create_assistants(&self, py: Python<'_>, text: &str, _blobs: Blobs) -> Encoded {
        let input = convert::read("assistant create request", text)?;
        let answer = py.detach(|| get_runtime().block_on(self.session.create(&input, false)));
        create_text(answer, &input)
    }

    fn acreate_assistants<'py>(&self, py: Python<'py>, text: &str, _blobs: Blobs) -> Reply<'py> {
        let input = convert::read("assistant create request", text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move {
            let answer = session.create(&input, true).await;
            create_text(answer, &input)
        })
    }

    fn create_thread(&self, py: Python<'_>, text: &str, _blobs: Blobs) -> Encoded {
        let request = convert::read("thread request", text)?;
        let answer = py.detach(|| {
            get_runtime().block_on(self.session.thread(ThreadKind::Create, &request, false))
        });
        thread_text(answer, &request.custom_llm_provider)
    }

    fn acreate_thread<'py>(&self, py: Python<'py>, text: &str, _blobs: Blobs) -> Reply<'py> {
        let request = convert::read("thread request", text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, thread_work(session, ThreadKind::Create, request))
    }

    fn get_thread(&self, py: Python<'_>, text: &str, _blobs: Blobs) -> Encoded {
        let request = convert::read("thread request", text)?;
        let answer = py.detach(|| {
            get_runtime().block_on(self.session.thread(ThreadKind::Get, &request, false))
        });
        thread_text(answer, &request.custom_llm_provider)
    }

    fn aget_thread<'py>(&self, py: Python<'py>, text: &str, _blobs: Blobs) -> Reply<'py> {
        let request = convert::read("thread request", text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, thread_work(session, ThreadKind::Get, request))
    }
}

// get_assistants and delete_assistant block this thread on the runtime with the GIL released.
fn read_delete_step(
    py: Python<'_>,
    session: &Session,
    input: &AssistantReadDeleteInput,
) -> PyResult<AssistantStep> {
    let provider = input.provider.clone();
    let answer = py.detach(|| get_runtime().block_on(session.read_delete(input)));
    answer
        .map_err(|error| family::read_delete(error, &provider, &session.redactor).into_exception())
}

// Their async twins await the same call on the asyncio loop's runtime.
async fn read_delete_work(
    session: Arc<Session>,
    input: AssistantReadDeleteInput,
    wanted: Answer,
) -> Encoded {
    let answer = session.read_delete(&input).await;
    let provider = input.provider;
    let step = answer.map_err(|error| {
        family::read_delete(error, &provider, &session.redactor).into_exception()
    })?;
    reply_text(step, wanted)
}

// The step must be the object the function returns; a reply of the other kind never leaves Rust.
fn reply_text(step: AssistantStep, wanted: Answer) -> Encoded {
    match (step, wanted) {
        (AssistantStep::Page(page), Answer::Page) => encode(&page),
        (AssistantStep::Deleted(deleted), Answer::Deleted) => encode(&deleted),
        (AssistantStep::Page(_), Answer::Deleted)
        | (AssistantStep::Deleted(_), Answer::Page)
        | (AssistantStep::Coroutine, Answer::Page | Answer::Deleted) => {
            Err(PyErr::from(CallError::Reply))
        }
    }
}

fn create_text(
    answer: Result<AssistantObject, Box<AssistantCreateFailure>>,
    input: &AssistantCreateInput,
) -> Encoded {
    let assistant = answer.map_err(|failure| {
        family::create(&failure, &input.model, &input.provider).into_exception()
    })?;
    encode(&assistant)
}

async fn thread_work(session: Arc<Session>, kind: ThreadKind, request: ThreadRequest) -> Encoded {
    let answer = session.thread(kind, &request, true).await;
    let provider = request.custom_llm_provider;
    thread_text(answer, &provider)
}

fn thread_text(answer: Result<Option<ThreadRecord>, ThreadOutcome>, provider: &str) -> Encoded {
    match answer {
        Ok(Some(record)) => encode(&record),
        Ok(None) => Ok(String::from("null")),
        Err(outcome) => Err(family::thread(&outcome, provider).into_exception()),
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
