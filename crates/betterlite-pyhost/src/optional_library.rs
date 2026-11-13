use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use pyo3::exceptions::PyModuleNotFoundError;
use pyo3::types::PyAnyMethods as _;
use pyo3::{Bound, Py, PyAny, PyErr, PyResult, Python};

use crate::blocking_work::WorkTracker;
use crate::call_error::{CallError, PythonError};
use crate::callable_handle::CallableHandle;
use crate::host::PythonHost;
use crate::json_values::{JsonObject, JsonValue};
use crate::owned_temp::{OwnedTempDir, TempError};

mod function;
mod tests;

#[derive(Debug, Default)]
pub struct OptionalLibrary {
    modules: Mutex<BTreeMap<String, Arc<Py<PyAny>>>>,
}

#[derive(Debug)]
pub struct LibraryModule {
    name: String,
    module: Arc<Py<PyAny>>,
}

#[derive(Debug)]
pub struct BlockingCall<'call> {
    pub function: &'call CallableHandle,
    pub args: Vec<JsonValue>,
    pub kwargs: JsonObject,
    // a new folder path becomes the last positional argument, so the caller orders the arguments
    pub new_temp_dir: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("{module} is not installed. Install it with: {install_hint}")]
    NotInstalled {
        module: String,
        install_hint: String,
    },
    #[error("importing the library failed: {source}")]
    Import {
        #[source]
        source: Box<PythonError>,
    },
    #[error("{module} has no attribute {path:?}")]
    MissingAttribute {
        module: String,
        path: String,
        #[source]
        source: Box<PythonError>,
    },
    #[error("reading {path:?} from {module} failed: {source}")]
    Attribute {
        module: String,
        path: String,
        #[source]
        source: Box<PythonError>,
    },
    #[error("{path:?} in {module} is not callable")]
    NotCallable { module: String, path: String },
    #[error("the call into {module} failed: {source}")]
    Call {
        module: String,
        #[source]
        source: Box<CallError>,
    },
    #[error(transparent)]
    Temp(#[from] TempError),
    #[error("the temporary folder {path:?} is not valid UTF-8, so Python cannot use it")]
    TempPathNotUtf8 { path: PathBuf },
}

impl OptionalLibrary {
    pub fn import(
        &self,
        host: &PythonHost,
        module_name: &str,
        install_hint: &str,
    ) -> Result<LibraryModule, LibraryError> {
        let cached = {
            let modules = self.lock_modules();
            modules.get(module_name).map(Arc::clone)
        };
        if let Some(module) = cached {
            return Ok(LibraryModule {
                name: module_name.to_owned(),
                module,
            });
        }
        let imported = host.attach(|py| {
            import_module(py, module_name)
                .map(Bound::unbind)
                .map_err(|error| import_error(py, module_name, install_hint, &error))
        })?;
        let loaded = Arc::new(imported);
        let stored = Arc::clone(
            self.lock_modules()
                .entry(module_name.to_owned())
                .or_insert_with(|| Arc::clone(&loaded)),
        );
        Ok(LibraryModule {
            name: module_name.to_owned(),
            module: stored,
        })
    }

    fn lock_modules(&self) -> MutexGuard<'_, BTreeMap<String, Arc<Py<PyAny>>>> {
        match self.modules.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl LibraryModule {
    pub fn function(&self, host: &PythonHost, path: &str) -> Result<CallableHandle, LibraryError> {
        let object = host.attach(|py| {
            let module = self.module.bind(py);
            function::resolve(py, module, &self.name, path)
        })?;
        CallableHandle::new(host, object).map_err(|source| LibraryError::Call {
            module: self.name.clone(),
            source: Box::new(source),
        })
    }

    pub async fn call_blocking(
        &self,
        host: &PythonHost,
        tracker: &WorkTracker,
        deadline: Duration,
        call: BlockingCall<'_>,
    ) -> Result<JsonValue, LibraryError> {
        let folder = call.new_temp_dir.then(OwnedTempDir::new).transpose()?;
        let mut args = call.args;
        if let Some(created) = &folder {
            let path = created
                .path()
                .to_str()
                .ok_or_else(|| LibraryError::TempPathNotUtf8 {
                    path: created.path().to_path_buf(),
                })?;
            args.push(JsonValue::String(path.to_owned()));
        }
        call.function
            .call(host, tracker, deadline, args, call.kwargs)
            .await
            .map_err(|source| LibraryError::Call {
                module: self.name.clone(),
                source: Box::new(source),
            })
    }
}

fn import_module<'py>(py: Python<'py>, module_name: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import("importlib")?
        .call_method1("import_module", (module_name,))
}

fn import_error(
    py: Python<'_>,
    module_name: &str,
    install_hint: &str,
    error: &PyErr,
) -> LibraryError {
    if missing_module(py, module_name, error) {
        return LibraryError::NotInstalled {
            module: module_name.to_owned(),
            install_hint: install_hint.to_owned(),
        };
    }
    LibraryError::Import {
        source: Box::new(PythonError::from_pyerr(py, error)),
    }
}

fn missing_module(py: Python<'_>, module_name: &str, error: &PyErr) -> bool {
    // a package that fails inside its __init__ raises a plain ImportError, so only a missing module counts
    if !error.is_instance_of::<PyModuleNotFoundError>(py) {
        return false;
    }
    error
        .value(py)
        .getattr("name")
        .and_then(|name| name.extract::<Option<String>>())
        .is_ok_and(|name| {
            name.is_some_and(|missing| {
                missing == module_name || module_name.starts_with(&format!("{missing}."))
            })
        })
}
