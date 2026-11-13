use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pyo3::{Py, PyAny, PyErr, Python};

use crate::blocking_work::{WorkError, WorkTracker};
use crate::call_error::{CallError, PythonError};
use crate::callable_handle::CallableHandle;
use crate::host::PythonHost;
use crate::instance_ref::{InstanceRef, RefError, RemoteModule};
use crate::remote_module::{ModuleFetcher, RemoteError, load_remote};

pub(crate) mod local_file;

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
        // the config load waits as long as an import can take
        self.load_within(value, config_file_path, LOAD_DEADLINE)
            .await
    }

    pub async fn load_within(
        &self,
        value: &str,
        config_file_path: Option<&Path>,
        deadline: Duration,
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
                self.load_local_value(module_name, instance_name, config, deadline)
                    .await?
            }
            InstanceRef::Remote(module) => self.load_remote_value(value, &module, deadline).await?,
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
        deadline: Duration,
    ) -> Result<Py<PyAny>, LoadError> {
        let job = self.tracker.run(&self.host, deadline, move |py| {
            local_file::load_local(py, &module_name, &instance_name, config.as_deref())
        });
        job.await.map_err(LoadError::Work)?
    }

    async fn load_remote_value(
        &self,
        url: &str,
        module: &RemoteModule,
        deadline: Duration,
    ) -> Result<Py<PyAny>, LoadError> {
        let job_url = url.to_owned();
        let job_module = module.clone();
        let fetcher = Arc::clone(&self.fetcher);
        let job = self.tracker.run(&self.host, deadline, move |py| {
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

mod tests;
