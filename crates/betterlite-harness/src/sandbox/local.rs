pub mod environment;
pub mod error;
mod files;
pub mod process;
mod state;
mod tests;
mod worker;

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::join_all;
use secrecy::{ExposeSecret, SecretString};

use crate::errors::HarnessError;
use crate::sandbox::{CompletedRun, Sandbox, SandboxCommand};
use crate::snapshots::snapshot_local;
use environment::filtered_environ;
use error::SandboxError;
use files::{path_string, read_file, write_file};
use process::SubprocessHandle;
use state::State;

pub struct LocalSandbox {
    workdir: String,
    environment: BTreeMap<String, SecretString>,
    temp_root: PathBuf,
    state: Arc<Mutex<State>>,
}

impl LocalSandbox {
    pub fn new(
        path: &Path,
        environment: &BTreeMap<String, SecretString>,
        temp_root: PathBuf,
    ) -> Result<Self, HarnessError> {
        let resolved = fs::canonicalize(path).map_err(|source| SandboxError::Root {
            path: path.to_path_buf(),
            source,
        })?;
        if !resolved.is_dir() {
            return Err(HarnessError::from(SandboxError::Root {
                path: resolved,
                source: io::Error::from(io::ErrorKind::NotADirectory),
            }));
        }
        Ok(Self {
            workdir: path_string(&resolved)?,
            environment: filtered_environ(environment, &BTreeMap::new()),
            temp_root: fs::canonicalize(temp_root)
                .map_err(|source| HarnessError::Sandbox { source })?,
            state: Arc::new(Mutex::new(State::default())),
        })
    }

    pub fn resolve_path(&self, path: &str) -> Result<String, HarnessError> {
        let resolved = self
            .state
            .lock()
            .map_err(|_poisoned| SandboxError::Poisoned)?
            .resolve(&self.workdir, path)?;
        path_string(&resolved)
    }

    #[must_use]
    pub fn child_env(
        &self,
        extra: &BTreeMap<String, SecretString>,
    ) -> BTreeMap<String, SecretString> {
        let mut result = self.environment.clone();
        result.extend(extra.clone());
        result
    }

    async fn access<Job, Output>(&self, job: Job) -> Result<Output, HarnessError>
    where
        Job: FnOnce(&mut State, &str) -> Result<Output, HarnessError> + Send + 'static,
        Output: Send + 'static,
    {
        let state = Arc::clone(&self.state);
        let root = self.workdir.clone();
        worker::blocking(move || {
            let mut locked = state.lock().map_err(|_poisoned| SandboxError::Poisoned)?;
            let result = job(&mut locked, &root);
            drop(locked);
            result
        })
        .await
    }
}

impl Sandbox for LocalSandbox {
    type Process = SubprocessHandle;

    fn workdir(&self) -> &str {
        &self.workdir
    }

    async fn exec(&self, command: &SandboxCommand) -> Result<Self::Process, HarnessError> {
        let request = command.clone();
        let environment = self.child_env(&command.env);
        self.access(move |state, root| {
            if state.closed {
                return Err(HarnessError::from(SandboxError::Closed));
            }
            if request.argv.is_empty() {
                return Err(HarnessError::from(SandboxError::EmptyCommand));
            }
            let cwd = state.resolve(root, request.cwd.as_deref().unwrap_or("."))?;
            if !cwd.is_dir() {
                return Err(HarnessError::from(SandboxError::Cwd(cwd)));
            }
            let handle = process::start(&request, &path_string(&cwd)?, &environment)?;
            state.processes.push(Arc::clone(&handle.child));
            Ok(handle)
        })
        .await
    }

    async fn run(
        &self,
        command: &SandboxCommand,
        timeout: Duration,
        max_output_bytes: usize,
    ) -> Result<CompletedRun, HarnessError> {
        let mut handle = self.exec(command).await?;
        let result = process::collect(&mut handle, command, timeout, max_output_bytes).await;
        self.access(move |state, _root| {
            state
                .processes
                .retain(|child| !Arc::ptr_eq(child, &handle.child));
            Ok(())
        })
        .await?;
        result
    }

    async fn read(&self, path: &str, max_bytes: usize) -> Result<Vec<u8>, HarnessError> {
        files::operation(self, path, move |resolved| read_file(resolved, max_bytes)).await
    }

    async fn write(&self, path: &str, data: &[u8]) -> Result<(), HarnessError> {
        let bytes = data.to_vec();
        files::operation(self, path, move |resolved| write_file(resolved, &bytes)).await
    }

    fn host_url(&self, port: u16) -> String {
        format!("http://127.0.0.1:{port}")
    }

    async fn which(&self, binary: &str) -> Result<Option<String>, HarnessError> {
        let environment = self.child_env(&BTreeMap::new());
        let name = binary.to_owned();
        let root = PathBuf::from(&self.workdir);
        worker::blocking(move || {
            let path = environment
                .get("PATH")
                .map_or("/bin:/usr/bin", ExposeSecret::expose_secret);
            match which::which_in(&name, Some(path), &root) {
                Ok(found) => path_string(&found).map(Some),
                Err(_not_found) => Ok(None),
            }
        })
        .await
    }

    async fn snapshot(&self) -> Result<BTreeMap<String, String>, HarnessError> {
        let root = self.access(|state, root| state.resolve(root, ".")).await?;
        snapshot_local(root)
            .await
            .map_err(|source| HarnessError::Sandbox {
                source: io::Error::other(source),
            })
    }

    async fn tempdir(&self) -> Result<String, HarnessError> {
        let root = self.temp_root.clone();
        self.access(move |state, _root| state.create_tempdir(&root))
            .await
    }

    async fn close(&self) -> Result<(), HarnessError> {
        let cleanup = self.access(|state, _root| Ok(state.take())).await?;
        join_all(cleanup.processes.iter().map(|child| child.kill())).await;
        worker::blocking(move || {
            drop(cleanup);
            Ok(())
        })
        .await
    }
}

impl Drop for LocalSandbox {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            drop(state.take());
        }
    }
}
