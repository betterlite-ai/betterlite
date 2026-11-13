mod args;
mod commands;
pub mod error;
mod files;
mod sandbox_impl;
mod state;
mod tests;

pub mod process;
pub mod snapshot;

use std::collections::BTreeMap;
use std::path::PathBuf;

use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::Mutex;

use crate::errors::{CappedOperation, HarnessError};
use args::{DEFAULT_WORKDIR, normalize_path, resolve_mounts};
use error::DockerSandboxError;
use state::State;

const DOCKER_HOST_ALIAS: &str = "host.docker.internal";
const MAX_HELPER_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

pub struct DockerSandbox {
    image: String,
    mounts: Vec<(String, String)>,
    workdir: String,
    env: Vec<(String, SecretString)>,
    name: Option<String>,
    docker_path: Option<PathBuf>,
    start_lock: Mutex<()>,
    state: Mutex<State>,
}

impl DockerSandbox {
    pub fn new(
        image: String,
        mounts: Vec<(String, String)>,
        workdir: Option<String>,
        env: Vec<(String, SecretString)>,
        name: Option<String>,
    ) -> Result<Self, HarnessError> {
        if image.is_empty() {
            return Err(HarnessError::from(DockerSandboxError::MissingImage));
        }
        let requested = workdir.unwrap_or_else(|| DEFAULT_WORKDIR.to_owned());
        if !requested.starts_with('/') {
            return Err(HarnessError::from(DockerSandboxError::WorkdirNotAbsolute {
                workdir: requested,
            }));
        }
        Ok(Self {
            image,
            mounts: resolve_mounts(mounts)?,
            workdir: normalize_path(&requested),
            env,
            name,
            docker_path: None,
            start_lock: Mutex::new(()),
            state: Mutex::new(State::default()),
        })
    }

    #[must_use]
    pub fn with_docker_program(mut self, program: PathBuf) -> Self {
        self.docker_path = Some(program);
        self
    }

    #[must_use]
    pub fn run_args(&self) -> Vec<String> {
        let mut argv = vec![
            "run".to_owned(),
            "-d".to_owned(),
            "--rm".to_owned(),
            format!("--add-host={DOCKER_HOST_ALIAS}:host-gateway"),
        ];
        if let Some(name) = &self.name {
            argv.push("--name".to_owned());
            argv.push(name.clone());
        }
        for (host, container) in &self.mounts {
            argv.push("-v".to_owned());
            argv.push(format!("{host}:{container}"));
        }
        for (name, value) in &self.env {
            argv.push("-e".to_owned());
            argv.push(format!("{name}={}", value.expose_secret()));
        }
        argv.push("-w".to_owned());
        argv.push(self.workdir.clone());
        argv.push(self.image.clone());
        argv.push("sleep".to_owned());
        argv.push("infinity".to_owned());
        argv
    }

    #[must_use]
    pub fn exec_args(
        &self,
        container_id: &str,
        cmd: &[String],
        env: &BTreeMap<String, SecretString>,
        cwd: Option<&str>,
    ) -> Vec<String> {
        let chosen = cwd
            .filter(|path| !path.is_empty())
            .unwrap_or(self.workdir.as_str());
        let mut argv = vec![
            "exec".to_owned(),
            "-i".to_owned(),
            "-w".to_owned(),
            self.container_path(chosen),
        ];
        for (name, value) in env {
            argv.push("-e".to_owned());
            argv.push(format!("{name}={}", value.expose_secret()));
        }
        argv.push(container_id.to_owned());
        argv.extend_from_slice(cmd);
        argv
    }

    #[must_use]
    pub fn container_path(&self, path: &str) -> String {
        if path.starts_with('/') {
            return normalize_path(path);
        }
        let separator = if self.workdir.ends_with('/') { "" } else { "/" };
        normalize_path(&format!("{}{separator}{path}", self.workdir))
    }

    pub async fn start(&self) -> Result<String, HarnessError> {
        if self.state.lock().await.closed {
            return Err(HarnessError::from(DockerSandboxError::Closed));
        }
        let _guard = self.start_lock.lock().await;
        if let Some(container_id) = &self.state.lock().await.container_id {
            return Ok(container_id.clone());
        }
        let mut client = process::start(&self.docker_program()?, &self.run_args())?;
        let (code, stdout, stderr) = process::finish(
            &mut client,
            None,
            MAX_HELPER_OUTPUT_BYTES,
            CappedOperation::Read,
        )
        .await?;
        if code != 0_i32 {
            return Err(HarnessError::from(DockerSandboxError::RunFailed {
                image: self.image.clone(),
                code,
                stderr: process::decode_utf8(&stderr).trim().to_owned(),
            }));
        }
        let container_id = process::decode_utf8(&stdout).trim().to_owned();
        if container_id.is_empty() {
            return Err(HarnessError::from(DockerSandboxError::NoContainerId));
        }
        self.state.lock().await.container_id = Some(container_id.clone());
        Ok(container_id)
    }

    async fn exec_capture(
        &self,
        command: Vec<String>,
        input: Option<&[u8]>,
        max: usize,
    ) -> Result<(i32, Vec<u8>, Vec<u8>), HarnessError> {
        let container_id = self.start().await?;
        let args = self.exec_args(&container_id, &command, &BTreeMap::new(), None);
        let mut client = process::start(&self.docker_program()?, &args)?;
        process::finish(&mut client, input, max, CappedOperation::Read).await
    }

    fn docker_program(&self) -> Result<PathBuf, DockerSandboxError> {
        if let Some(program) = &self.docker_path {
            return Ok(program.clone());
        }
        which::which("docker").map_err(|_not_found| DockerSandboxError::DockerMissing)
    }
}
