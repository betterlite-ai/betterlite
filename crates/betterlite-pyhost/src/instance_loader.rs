use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pyo3::{Py, PyAny, PyErr, Python};

use crate::blocking_work::{WorkError, WorkTracker};
use crate::call_error::{CallError, PythonError};
use crate::callable_handle::CallableHandle;
use crate::host::PythonHost;
use crate::instance_ref::{InstanceRef, RemoteModule};
use crate::remote_module::{ModuleFetcher, RemoteError, load_remote};

const LOAD_DEADLINE: Duration = Duration::from_secs(60);

pub(crate) mod local_file;

#[derive(Debug, thiserror::Error)]
pub enum LoadError<E: Error> {
    #[error(
        "Remote module loading (s3://, gcs://) is only permitted from the config-file load path. Register the module under ``litellm_settings`` in your config.yaml instead."
    )]
    RemoteNeedsConfigPath,
    #[error(transparent)]
    Local(#[from] LocalError),
    #[error("Failed to load custom logger from {url}: {source}")]
    Remote {
        url: String,
        #[source]
        source: Box<RemoteError<E>>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum LocalError {
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

pub struct InstanceLoader<E: Error + Send + Sync + 'static> {
    host: PythonHost,
    fetcher: Arc<dyn ModuleFetcher<Error = E>>,
}

struct LocalJob {
    module_name: String,
    instance_name: String,
    config: Option<PathBuf>,
}

impl<E: Error + Send + Sync + 'static> InstanceLoader<E> {
    #[must_use]
    pub fn new(host: PythonHost, fetcher: Arc<dyn ModuleFetcher<Error = E>>) -> Self {
        // the fetcher is a trait object, so the loader never names the downloading crate
        Self { host, fetcher }
    }

    pub async fn load(
        &self,
        tracker: &WorkTracker,
        value: &str,
        config_file_path: Option<&Path>,
    ) -> Result<LoadedObject, LoadError<E>> {
        // the config load waits as long as an import can take
        self.load_within(tracker, value, config_file_path, LOAD_DEADLINE)
            .await
    }

    pub async fn load_within(
        &self,
        tracker: &WorkTracker,
        value: &str,
        config_file_path: Option<&Path>,
        deadline: Duration,
    ) -> Result<LoadedObject, LoadError<E>> {
        let remote = value.starts_with("s3://") || value.starts_with("gcs://");
        if remote && config_file_path.is_none() {
            return Err(LoadError::RemoteNeedsConfigPath);
        }
        let reference = InstanceRef::parse(value).map_err(|source| LoadError::Remote {
            url: value.to_owned(),
            source: Box::new(RemoteError::Reference(source)),
        })?;
        let config = config_file_path.map(Path::to_path_buf);
        let object = match reference {
            InstanceRef::Local {
                module_name,
                instance_name,
            } => {
                let local = LocalJob {
                    module_name,
                    instance_name,
                    config,
                };
                self.load_local_value(local, tracker, deadline).await?
            }
            InstanceRef::Remote(module) => {
                self.load_remote_value(tracker, value, &module, deadline)
                    .await?
            }
        };
        Ok(LoadedObject {
            object,
            value: value.to_owned(),
        })
    }

    async fn load_local_value(
        &self,
        local: LocalJob,
        tracker: &WorkTracker,
        deadline: Duration,
    ) -> Result<Py<PyAny>, LoadError<E>> {
        let running = tracker.run(&self.host, deadline, move |py| {
            local_file::load_local(
                py,
                &local.module_name,
                &local.instance_name,
                local.config.as_deref(),
            )
        });
        match running.await {
            Ok(result) => result.map_err(LoadError::Local),
            Err(work) => Err(LoadError::Local(LocalError::Work(work))),
        }
    }

    async fn load_remote_value(
        &self,
        tracker: &WorkTracker,
        url: &str,
        module: &RemoteModule,
        deadline: Duration,
    ) -> Result<Py<PyAny>, LoadError<E>> {
        let job_url = url.to_owned();
        let job_module = module.clone();
        let fetcher = Arc::clone(&self.fetcher);
        let job = tracker.run(&self.host, deadline, move |py| {
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

mod tests;
