use std::path::PathBuf;

use pyo3::exceptions::PyImportError;
use pyo3::types::PyAnyMethods as _;
use pyo3::{Bound, Py, PyAny, PyErr, Python};
use tracing::{debug, info};

use crate::blocking_work::WorkError;
use crate::instance_loader::{PythonFailure, module_spec};
use crate::instance_ref::{RefError, RemoteModule, Storage};
use crate::owned_temp::{OwnedTempDir, TempError};

const MODULE_FILE_NAME: &str = "module.py";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct FetchError {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleRequest {
    pub storage: Storage,
    pub bucket: String,
    pub object_key: String,
    pub local_path: PathBuf,
}

pub trait ModuleFetcher: Send + Sync {
    fn fetch(&self, request: &ModuleRequest) -> Result<(), FetchError>;
}

#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    #[error(transparent)]
    Reference(#[from] RefError),
    #[error("Failed to download {object_key} from {scheme} bucket {bucket}")]
    Fetch {
        scheme: &'static str,
        bucket: String,
        object_key: String,
        #[source]
        source: FetchError,
    },
    #[error(transparent)]
    Python(#[from] PythonFailure),
    #[error(transparent)]
    Temp(#[from] TempError),
    #[error("the downloaded module file {path:?} is not valid UTF-8, so Python cannot import it")]
    PathNotUtf8 { path: PathBuf },
    #[error(transparent)]
    Work(#[from] WorkError),
}

pub(crate) fn load_remote(
    py: Python<'_>,
    fetcher: &dyn ModuleFetcher,
    url: &str,
    module: &RemoteModule,
) -> Result<Py<PyAny>, RemoteError> {
    debug!(
        storage = module.storage.as_str(),
        bucket = %module.bucket,
        object_key = %module.object_key,
        instance = %module.instance_name,
        "loading a custom logger module from a bucket"
    );
    let directory = OwnedTempDir::new()?;
    let local_path = directory.file(MODULE_FILE_NAME)?;
    let request = ModuleRequest {
        storage: module.storage,
        bucket: module.bucket.clone(),
        object_key: module.object_key.clone(),
        local_path: local_path.clone(),
    };
    if let Err(source) = py.detach(|| fetcher.fetch(&request)) {
        return Err(RemoteError::Fetch {
            scheme: module.storage.as_str(),
            bucket: module.bucket.clone(),
            object_key: module.object_key.clone(),
            source,
        });
    }
    let location =
        local_path
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| RemoteError::PathNotUtf8 {
                path: local_path.clone(),
            })?;
    let object = execute_remote(py, module, &location)
        .map_err(|error| PythonFailure::capture(py, &error))?;
    info!(url = %url, "loaded a custom logger module from a bucket");
    Ok(object)
}

fn execute_remote(
    py: Python<'_>,
    module: &RemoteModule,
    location: &str,
) -> Result<Py<PyAny>, PyErr> {
    let spec = module_spec(py, &module.module_path, location)?;
    if spec.is_none() {
        return Err(PyImportError::new_err(format!(
            "Could not create module spec for {location}"
        )));
    }
    let loader = spec.getattr("loader")?;
    if loader.is_none() {
        return Err(PyImportError::new_err(format!(
            "Could not create module spec for {location}"
        )));
    }
    let util = py.import("importlib.util")?;
    let built = util.call_method1("module_from_spec", (&spec,))?;
    loader.call_method1("exec_module", (&built,))?;
    built.getattr(&module.instance_name).map(Bound::unbind)
}

mod tests;
