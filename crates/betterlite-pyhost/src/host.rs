use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pyo3::types::PyAny;
use pyo3::types::PyAnyMethods as _;
use pyo3::{Bound, PyErr, Python};

pub const DEFAULT_SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub module_dirs: Vec<PathBuf>,
    pub shutdown_grace: Duration,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            module_dirs: Vec::new(),
            shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("Python {found:?} is too old; betterlite needs 3.10 or newer")]
    BadPythonVersion { found: (u8, u8, u8) },
    #[error("the module folder {path:?} is not valid UTF-8, so Python cannot import from it")]
    ModuleDirNotUtf8 { path: PathBuf },
    #[error("Python could not import {module}: {source}")]
    Import {
        module: String,
        #[source]
        source: PyErr,
    },
    #[error("the Python interpreter failed: {source}")]
    Python {
        #[source]
        source: PyErr,
    },
}

#[derive(Debug, Clone)]
pub struct PythonHost {
    state: Arc<HostState>,
}

#[derive(Debug)]
struct HostState {
    version: (u8, u8, u8),
}

impl PythonHost {
    pub fn start(config: &HostConfig) -> Result<Self, HostError> {
        let version = Python::attach(|py| {
            let info = py.version_info();
            let version = (info.major, info.minor, info.patch);
            ensure_supported(version)?;
            place_module_dirs(py, &config.module_dirs)?;
            Ok(version)
        })?;
        Ok(Self {
            state: Arc::new(HostState { version }),
        })
    }

    #[must_use]
    pub fn version(&self) -> (u8, u8, u8) {
        // pyo3 never finalizes the interpreter after start, so the recorded triple stays right.
        self.state.version
    }

    pub fn attach<T, F>(&self, body: F) -> T
    where
        F: for<'py> FnOnce(Python<'py>) -> T,
    {
        // The Arc is the proof the interpreter is up; pyo3 attaches to that one and never finalizes it.
        Python::attach(body)
    }

    pub fn import_check(&self, module: &str) -> Result<(), HostError> {
        self.attach(|py| match py.import(module) {
            Ok(_) => Ok(()),
            Err(source) => Err(HostError::Import {
                module: module.to_owned(),
                source,
            }),
        })
    }
}

fn ensure_supported(version: (u8, u8, u8)) -> Result<(), HostError> {
    if version < (3, 10, 0) {
        return Err(HostError::BadPythonVersion { found: version });
    }
    Ok(())
}

fn place_module_dirs(py: Python<'_>, module_dirs: &[PathBuf]) -> Result<(), HostError> {
    let sys = py
        .import("sys")
        .map_err(|source| HostError::Python { source })?;
    let path = sys
        .getattr("path")
        .map_err(|source| HostError::Python { source })?;
    let mut wanted: Vec<&str> = Vec::with_capacity(module_dirs.len());
    for dir in module_dirs {
        let text = dir
            .to_str()
            .ok_or_else(|| HostError::ModuleDirNotUtf8 { path: dir.clone() })?;
        wanted.push(text);
    }
    for (index, dir) in wanted.iter().copied().enumerate() {
        while listed_on_path(&path, dir)? {
            path.call_method1("remove", (dir,))
                .map_err(|source| HostError::Python { source })?;
        }
        path.call_method1("insert", (index, dir))
            .map_err(|source| HostError::Python { source })?;
    }
    Ok(())
}

fn listed_on_path(path: &Bound<'_, PyAny>, dir: &str) -> Result<bool, HostError> {
    let count = path
        .call_method1("count", (dir,))
        .and_then(|value| value.extract::<usize>())
        .map_err(|source| HostError::Python { source })?;
    Ok(count > 0)
}

mod tests;
