use std::path::{Path, PathBuf, is_separator};
use std::sync::Arc;
use std::time::Duration;

use pyo3::exceptions::PyImportError;
use pyo3::types::PyAnyMethods as _;
use pyo3::{Bound, Py, PyAny, PyErr, Python};

use crate::blocking_work::{WorkError, WorkTracker};
use crate::call_error::{CallError, PythonError};
use crate::callable_handle::CallableHandle;
use crate::host::PythonHost;
use crate::instance_ref::{InstanceRef, RefError, RemoteModule};
use crate::remote_module::{ModuleFetcher, RemoteError, load_remote};

const LOAD_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Reference(#[from] RefError),
    #[error(
        "Remote module loading (s3://, gcs://) is only permitted from the config-file load path. Register the module under ``litellm_settings`` in your config.yaml instead."
    )]
    RemoteNeedsConfigPath,
    #[error("Could not import {instance_name} from {module_name}")]
    Import {
        module_name: String,
        instance_name: String,
        #[source]
        source: Box<PythonError>,
    },
    #[error("Could not import {instance_name} from {module_name}")]
    UnsafeModulePath {
        module_name: String,
        instance_name: String,
    },
    #[error(transparent)]
    Python(Box<PythonFailure>),
    #[error("Failed to load custom logger from {url}: {source}")]
    Remote {
        url: String,
        #[source]
        source: Box<RemoteError>,
    },
    #[error(transparent)]
    Work(#[from] WorkError),
    #[error("the module file {path:?} is not valid UTF-8, so Python cannot import it")]
    PathNotUtf8 { path: PathBuf },
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct PythonFailure {
    pub type_name: String,
    pub message: String,
    #[source]
    pub source: Box<PythonError>,
}

impl PythonFailure {
    pub(crate) fn capture(py: Python<'_>, error: &PyErr) -> Self {
        let source = PythonError::from_pyerr(py, error);
        Self {
            type_name: source.type_name.clone(),
            message: source.message.clone(),
            source: Box::new(source),
        }
    }
}

#[derive(Debug)]
pub struct LoadedObject {
    pub object: Py<PyAny>,
    pub value: String,
}

impl LoadedObject {
    pub fn handle(&self, host: &PythonHost) -> Result<CallableHandle, CallError> {
        // a Py clone needs the GIL, so the object is copied inside attach
        let object = host.attach(|py| self.object.clone_ref(py));
        CallableHandle::new(host, object)
    }
}

pub struct InstanceLoader {
    host: PythonHost,
    fetcher: Arc<dyn ModuleFetcher>,
    tracker: WorkTracker,
}

impl InstanceLoader {
    #[must_use]
    pub fn new(host: PythonHost, fetcher: Arc<dyn ModuleFetcher>) -> Self {
        // the loader runs user imports off the async workers through a pool of its own
        Self {
            host,
            fetcher,
            tracker: WorkTracker::default(),
        }
    }

    pub async fn load(
        &self,
        value: &str,
        config_file_path: Option<&Path>,
    ) -> Result<LoadedObject, LoadError> {
        let remote = value.starts_with("s3://") || value.starts_with("gcs://");
        if remote && config_file_path.is_none() {
            return Err(LoadError::RemoteNeedsConfigPath);
        }
        let reference = InstanceRef::parse(value).map_err(|error| wrap_reference(error, value))?;
        let config = config_file_path.map(Path::to_path_buf);
        let object = match reference {
            InstanceRef::Local {
                module_name,
                instance_name,
            } => {
                self.load_local_value(module_name, instance_name, config)
                    .await?
            }
            InstanceRef::Remote(module) => self.load_remote_value(value, &module).await?,
        };
        Ok(LoadedObject {
            object,
            value: value.to_owned(),
        })
    }

    async fn load_local_value(
        &self,
        module_name: String,
        instance_name: String,
        config: Option<PathBuf>,
    ) -> Result<Py<PyAny>, LoadError> {
        let job = self.tracker.run(&self.host, LOAD_DEADLINE, move |py| {
            load_local(py, &module_name, &instance_name, config.as_deref())
        });
        job.await.map_err(LoadError::Work)?
    }

    async fn load_remote_value(
        &self,
        url: &str,
        module: &RemoteModule,
    ) -> Result<Py<PyAny>, LoadError> {
        let job_url = url.to_owned();
        let job_module = module.clone();
        let fetcher = Arc::clone(&self.fetcher);
        let job = self.tracker.run(&self.host, LOAD_DEADLINE, move |py| {
            load_remote(py, fetcher.as_ref(), &job_url, &job_module)
        });
        match job.await {
            Ok(Ok(object)) => Ok(object),
            Ok(Err(source)) => Err(LoadError::Remote {
                url: url.to_owned(),
                source: Box::new(source),
            }),
            Err(work) => Err(LoadError::Remote {
                url: url.to_owned(),
                source: Box::new(RemoteError::Work(work)),
            }),
        }
    }
}

fn wrap_reference(error: LoadError, value: &str) -> LoadError {
    let LoadError::Reference(source) = error else {
        return error;
    };
    LoadError::Remote {
        url: value.to_owned(),
        source: Box::new(RemoteError::Reference(source)),
    }
}

fn load_local(
    py: Python<'_>,
    module_name: &str,
    instance_name: &str,
    config_file_path: Option<&Path>,
) -> Result<Py<PyAny>, LoadError> {
    let file = local_module_file(config_file_path, module_name, instance_name)?;
    let module = import_module(py, module_name, file.as_deref())
        .map_err(|error| local_error(py, &error, module_name, instance_name))?;
    let instance = module
        .getattr(instance_name)
        .map_err(|error| local_error(py, &error, module_name, instance_name))?;
    Ok(instance.unbind())
}

fn local_module_file(
    config_file_path: Option<&Path>,
    module_name: &str,
    instance_name: &str,
) -> Result<Option<String>, LoadError> {
    let Some(config) = config_file_path else {
        return Ok(None);
    };
    if module_name.is_empty() {
        return Ok(None);
    }
    let segments = module_segments(module_name).ok_or_else(|| LoadError::UnsafeModulePath {
        module_name: module_name.to_owned(),
        instance_name: instance_name.to_owned(),
    })?;
    let mut path = config.parent().unwrap_or(config).to_path_buf();
    for segment in segments {
        path.push(segment);
    }
    let mut name = path.into_os_string();
    name.push(".py");
    let file = PathBuf::from(name);
    if !file.exists() {
        return Ok(None);
    }
    let Some(location) = file.to_str() else {
        return Err(LoadError::PathNotUtf8 { path: file });
    };
    Ok(Some(location.to_owned()))
}

fn module_segments(module_name: &str) -> Option<Vec<&str>> {
    module_name
        .split('.')
        .map(|segment| plain_segment(segment).then_some(segment))
        .collect()
}

fn plain_segment(segment: &str) -> bool {
    // a part that starts with a separator would make Path::push replace the config folder
    !segment.is_empty() && !segment.starts_with(is_separator)
}

fn import_module<'py>(
    py: Python<'py>,
    module_name: &str,
    file: Option<&str>,
) -> Result<Bound<'py, PyAny>, PyErr> {
    if let Some(location) = file {
        return load_module_file(py, module_name, location);
    }
    let importlib = py.import("importlib")?;
    importlib.call_method1("import_module", (module_name,))
}

fn load_module_file<'py>(
    py: Python<'py>,
    module_name: &str,
    location: &str,
) -> Result<Bound<'py, PyAny>, PyErr> {
    let spec = module_spec(py, module_name, location)?;
    if spec.is_none() {
        return Err(PyImportError::new_err(format!(
            "Could not find a module specification for {location}"
        )));
    }
    let util = py.import("importlib.util")?;
    let module = util.call_method1("module_from_spec", (&spec,))?;
    let loader = spec.getattr("loader")?;
    if loader.is_none() {
        return Err(PyImportError::new_err(format!(
            "Could not find a module loader for {location}"
        )));
    }
    loader.call_method1("exec_module", (&module,))?;
    Ok(module)
}

pub(crate) fn module_spec<'py>(
    py: Python<'py>,
    module_name: &str,
    location: &str,
) -> Result<Bound<'py, PyAny>, PyErr> {
    py.import("importlib.util")?
        .call_method1("spec_from_file_location", (module_name, location))
}

fn local_error(py: Python<'_>, error: &PyErr, module_name: &str, instance_name: &str) -> LoadError {
    if !module_name.is_empty()
        && !instance_name.is_empty()
        && error.is_instance_of::<PyImportError>(py)
    {
        return LoadError::Import {
            module_name: module_name.to_owned(),
            instance_name: instance_name.to_owned(),
            source: Box::new(PythonError::from_pyerr(py, error)),
        };
    }
    LoadError::Python(Box::new(PythonFailure::capture(py, error)))
}

mod tests;
