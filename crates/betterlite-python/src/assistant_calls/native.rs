use std::sync::Arc;

use betterlite_core::api::assistant_read_delete::AssistantStep;
use betterlite_openai::threads::records::ThreadRecord;
use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{future_into_py, get_runtime};

use super::convert;
use super::error::{CallError, family};
use super::session::{Session, ThreadKind, ThreadOutcome};

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

    fn get_assistants(
        &self,
        py: Python<'_>,
        payload_text: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<String> {
        let input = convert::request_read_delete(payload_text)?;
        let provider = input.provider.clone();
        let answer = py.detach(|| get_runtime().block_on(self.session.read_delete(&input)));
        let step = answer.map_err(|error| {
            family::read_delete(error, &provider, &self.session.redactor).into_exception()
        })?;
        page_text(step)
    }

    fn aget_assistants<'py>(
        &self,
        py: Python<'py>,
        payload_text: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let input = convert::request_read_delete(payload_text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move {
            let provider = input.provider.clone();
            let answer = session.read_delete(&input).await;
            let step = answer.map_err(|error| {
                family::read_delete(error, &provider, &session.redactor).into_exception()
            })?;
            page_text(step).map_err(CallError::into_exception)
        })
    }

    fn delete_assistant(
        &self,
        py: Python<'_>,
        payload_text: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<String> {
        let input = convert::request_read_delete(payload_text)?;
        let provider = input.provider.clone();
        let answer = py.detach(|| get_runtime().block_on(self.session.read_delete(&input)));
        let step = answer.map_err(|error| {
            family::read_delete(error, &provider, &self.session.redactor).into_exception()
        })?;
        deleted_text(step)
    }

    fn adelete_assistant<'py>(
        &self,
        py: Python<'py>,
        payload_text: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let input = convert::request_read_delete(payload_text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move {
            let provider = input.provider.clone();
            let answer = session.read_delete(&input).await;
            let step = answer.map_err(|error| {
                family::read_delete(error, &provider, &session.redactor).into_exception()
            })?;
            deleted_text(step).map_err(CallError::into_exception)
        })
    }

    fn create_assistants(
        &self,
        py: Python<'_>,
        payload_text: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<String> {
        let input = convert::request_create(payload_text)?;
        let model = input.model.clone();
        let provider = input.provider.clone();
        let answer = py.detach(|| get_runtime().block_on(self.session.create(&input, false)));
        let assistant = answer
            .map_err(|failure| family::create(&failure, &model, &provider).into_exception())?;
        convert::write(&assistant).map_err(CallError::into_exception)
    }

    fn acreate_assistants<'py>(
        &self,
        py: Python<'py>,
        payload_text: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let input = convert::request_create(payload_text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move {
            let model = input.model.clone();
            let provider = input.provider.clone();
            let answer = session.create(&input, true).await;
            let assistant = answer
                .map_err(|failure| family::create(&failure, &model, &provider).into_exception())?;
            convert::write(&assistant).map_err(CallError::into_exception)
        })
    }

    fn create_thread(
        &self,
        py: Python<'_>,
        payload_text: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<String> {
        let request = convert::request_thread(payload_text)?;
        let provider = request.custom_llm_provider.clone();
        let answer = py.detach(|| {
            get_runtime().block_on(self.session.thread(ThreadKind::Create, &request, false))
        });
        thread_text(answer, &provider)
    }

    fn acreate_thread<'py>(
        &self,
        py: Python<'py>,
        payload_text: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = convert::request_thread(payload_text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move {
            let provider = request.custom_llm_provider.clone();
            let answer = session.thread(ThreadKind::Create, &request, true).await;
            thread_text(answer, &provider).map_err(CallError::into_exception)
        })
    }

    fn get_thread(
        &self,
        py: Python<'_>,
        payload_text: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<String> {
        let request = convert::request_thread(payload_text)?;
        let provider = request.custom_llm_provider.clone();
        let answer = py.detach(|| {
            get_runtime().block_on(self.session.thread(ThreadKind::Get, &request, false))
        });
        thread_text(answer, &provider)
    }

    fn aget_thread<'py>(
        &self,
        py: Python<'py>,
        payload_text: &str,
        _blobs: Vec<Vec<u8>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let request = convert::request_thread(payload_text)?;
        let session = Arc::clone(&self.session);
        future_into_py(py, async move {
            let provider = request.custom_llm_provider.clone();
            let answer = session.thread(ThreadKind::Get, &request, true).await;
            thread_text(answer, &provider).map_err(CallError::into_exception)
        })
    }
}

fn page_text(step: AssistantStep) -> PyResult<String> {
    match step {
        AssistantStep::Page(page) => convert::write(&page).map_err(CallError::into_exception),
        AssistantStep::Deleted(_) | AssistantStep::Coroutine => {
            Err(CallError::Reply.into_exception())
        }
    }
}

fn deleted_text(step: AssistantStep) -> PyResult<String> {
    match step {
        AssistantStep::Deleted(deleted) => {
            convert::write(&deleted).map_err(CallError::into_exception)
        }
        AssistantStep::Page(_) | AssistantStep::Coroutine => Err(CallError::Reply.into_exception()),
    }
}

fn thread_text(
    answer: Result<Option<ThreadRecord>, ThreadOutcome>,
    provider: &str,
) -> PyResult<String> {
    match answer {
        Ok(Some(record)) => convert::write(&record).map_err(CallError::into_exception),
        Ok(None) => Ok(String::from("null")),
        Err(ThreadOutcome::Sync(error)) => {
            Err(family::thread_sync(&error, provider).into_exception())
        }
        Err(ThreadOutcome::Async(failure)) => {
            Err(family::thread_async(&failure, provider).into_exception())
        }
    }
}
