use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_pyhost::blocking_work::WorkTracker;
use betterlite_pyhost::call_error::CallError;
use betterlite_pyhost::callable_handle::CallableHandle;
use betterlite_pyhost::host::PythonHost;
use betterlite_pyhost::optional_library::{LibraryError, OptionalLibrary};
use pyo3::types::{PyAnyMethods as _, PyString};
use pyo3::{Bound, IntoPyObjectExt as _, Py, PyAny, PyErr, Python};

use super::session::{
    RunResult, SandboxSession, SdkFailure, SessionError, SessionOpener, SessionOptions,
};

const SDK_MODULE: &str = "llm_sandbox";
const SDK_INSTALL_HINT: &str = "llm-sandbox";
const SESSION_CLASS: &str = "SandboxSession";

pub struct SdkSessions {
    pub host: PythonHost,
    pub tracker: Arc<WorkTracker>,
    pub library: OptionalLibrary,
    pub deadline: Duration,
}

#[async_trait]
impl SessionOpener for SdkSessions {
    async fn open(
        &self,
        options: &SessionOptions,
    ) -> Result<Box<dyn SandboxSession>, SessionError> {
        let module = self
            .library
            .import(&self.host, SDK_MODULE, SDK_INSTALL_HINT)
            .map_err(|source| open_failure(&source))?;
        let class = module
            .function(&self.host, SESSION_CLASS)
            .map_err(|source| open_failure(&source))?;
        let keywords = self.host.attach(|py| keywords_of(py, options))?;
        let created = class
            .call_raw(
                &self.host,
                &self.tracker,
                self.deadline,
                Vec::new(),
                keywords,
            )
            .await
            .map_err(|source| call_failure(SESSION_CLASS, &source))?;
        let manager = CallableHandle::new(&self.host, created)
            .map_err(|source| call_failure(SESSION_CLASS, &source))?;
        let session = enter(&self.host, &self.tracker, self.deadline, &manager).await?;
        Ok(Box::new(SdkSession {
            host: self.host.clone(),
            tracker: Arc::clone(&self.tracker),
            deadline: self.deadline,
            manager,
            session,
            closed: AtomicBool::new(false),
        }))
    }
}

async fn enter(
    host: &PythonHost,
    tracker: &Arc<WorkTracker>,
    deadline: Duration,
    manager: &CallableHandle,
) -> Result<CallableHandle, SessionError> {
    let entering = manager
        .attr(host, "__enter__")
        .map_err(|source| call_failure("__enter__", &source))?;
    let entered = entering
        .call_raw(host, tracker, deadline, Vec::new(), BTreeMap::new())
        .await
        .map_err(|source| call_failure("__enter__", &source))?;
    CallableHandle::new(host, entered).map_err(|source| call_failure("__enter__", &source))
}

struct SdkSession {
    host: PythonHost,
    tracker: Arc<WorkTracker>,
    deadline: Duration,
    manager: CallableHandle,
    session: CallableHandle,
    closed: AtomicBool,
}

impl SdkSession {
    async fn copy_paths(
        &self,
        operation: &'static str,
        source: &str,
        target: &str,
    ) -> Result<(), SessionError> {
        self.invoke(operation, &[source, target]).await?;
        Ok(())
    }

    async fn invoke(
        &self,
        operation: &'static str,
        arguments: &[&str],
    ) -> Result<Py<PyAny>, SessionError> {
        let optionals = self.host.attach(|py| {
            arguments
                .iter()
                .map(|text| PyString::new(py, text).into_any().unbind())
                .collect::<Vec<Py<PyAny>>>()
        });
        let handle = self
            .session
            .attr(&self.host, operation)
            .map_err(|source| call_failure(operation, &source))?;
        handle
            .call_raw(
                &self.host,
                &self.tracker,
                self.deadline,
                optionals,
                BTreeMap::new(),
            )
            .await
            .map_err(|source| call_failure(operation, &source))
    }
}

fn path_text<'path>(
    operation: &'static str,
    path: &'path Path,
) -> Result<&'path str, SessionError> {
    path.to_str().ok_or_else(|| SessionError::Call {
        operation,
        source: SdkFailure::from(format!("the {operation} path is not valid UTF-8")),
    })
}

#[async_trait]
impl SandboxSession for SdkSession {
    async fn copy_to_runtime(&self, local: &Path, sandbox_path: &str) -> Result<(), SessionError> {
        let text = path_text("copy_to_runtime", local)?;
        self.copy_paths("copy_to_runtime", text, sandbox_path).await
    }

    async fn copy_from_runtime(
        &self,
        source: &str,
        destination: &Path,
    ) -> Result<(), SessionError> {
        let text = path_text("copy_from_runtime", destination)?;
        self.copy_paths("copy_from_runtime", source, text).await
    }

    async fn run(&self, code: &str) -> Result<RunResult, SessionError> {
        let value = self.invoke("run", &[code]).await?;
        self.host.attach(|py| read_result(value.bind(py)))
    }

    async fn close(&self) -> Result<(), SessionError> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let handle = self
            .manager
            .attr(&self.host, "__exit__")
            .map_err(|source| call_failure("__exit__", &source))?;
        let arguments = self.host.attach(|py| vec![py.None(), py.None(), py.None()]);
        handle
            .call_raw(
                &self.host,
                &self.tracker,
                self.deadline,
                arguments,
                BTreeMap::new(),
            )
            .await
            .map_err(|source| call_failure("__exit__", &source))?;
        Ok(())
    }
}

impl Drop for SdkSession {
    fn drop(&mut self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        // a cancelled future cannot await, so the last-resort close runs on this thread
        let outcome = self.host.attach(|py| {
            self.manager
                .object
                .bind(py)
                .call_method1("__exit__", (py.None(), py.None(), py.None()))
                .map(Bound::unbind)
        });
        if let Err(error) = outcome {
            tracing::warn!("the sandbox session could not be closed: {error}");
        }
    }
}

fn keywords_of(
    py: Python<'_>,
    options: &SessionOptions,
) -> Result<BTreeMap<String, Py<PyAny>>, SessionError> {
    let mut keywords = BTreeMap::new();
    let language = PyString::new(py, options.lang).into_any().unbind();
    keywords.insert("lang".to_owned(), language);
    let verbose = options
        .verbose
        .into_py_any(py)
        .map_err(|source| start_failure(source.to_string()))?;
    keywords.insert("verbose".to_owned(), verbose);
    if let Some(image) = &options.image {
        keywords.insert(
            "image".to_owned(),
            PyString::new(py, image).into_any().unbind(),
        );
    }
    Ok(keywords)
}

fn read_result(result: &Bound<'_, PyAny>) -> Result<RunResult, SessionError> {
    let exit_code = result
        .getattr("exit_code")
        .and_then(|value| value.extract::<i32>())
        .map_err(|source| unreadable("exit_code", &source))?;
    Ok(RunResult {
        exit_code,
        stdout: read_text(result, "stdout")?,
        stderr: read_text(result, "stderr")?,
    })
}

fn read_text(result: &Bound<'_, PyAny>, field: &str) -> Result<Option<String>, SessionError> {
    result
        .getattr(field)
        .and_then(|value| value.extract::<Option<String>>())
        .map_err(|source| unreadable(field, &source))
}

fn unreadable(field: &str, source: &PyErr) -> SessionError {
    SessionError::Call {
        operation: "run",
        source: SdkFailure::from(format!("the {field} field: {source}")),
    }
}

pub(super) fn open_failure(error: &LibraryError) -> SessionError {
    if let LibraryError::NotInstalled { .. } = error {
        return SessionError::SdkMissing;
    }
    start_failure(error.to_string())
}

fn start_failure(message: String) -> SessionError {
    SessionError::Start {
        source: SdkFailure::from(message),
    }
}

fn call_failure(operation: &'static str, source: &CallError) -> SessionError {
    SessionError::Call {
        operation,
        source: SdkFailure::from(source.to_string()),
    }
}
