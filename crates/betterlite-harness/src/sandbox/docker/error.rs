use std::io;
use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

use crate::errors::HarnessError;

#[derive(Debug, Error)]
pub enum DockerSandboxError {
    #[error("docker sandbox needs an image")]
    MissingImage,
    #[error("docker workdir must be absolute: {workdir}")]
    WorkdirNotAbsolute { workdir: String },
    #[error("path is not UTF-8: {}", path.display())]
    PathEncoding { path: PathBuf },
    #[error(
        "docker sandbox requires the `docker` CLI on PATH; install Docker or use sandbox.local(path)"
    )]
    DockerMissing,
    #[error("could not run docker: {source}")]
    Spawn {
        #[source]
        source: io::Error,
    },
    #[error("sandbox is closed")]
    Closed,
    #[error("sandbox state poisoned")]
    Poisoned,
    #[error("docker run {image} failed ({code}): {stderr}")]
    RunFailed {
        image: String,
        code: i32,
        stderr: String,
    },
    #[error("docker run returned no container id")]
    NoContainerId,
    #[error("exec() needs a non-empty command")]
    EmptyCommand,
    #[error("command timed out after {timeout:?}: {name}")]
    Timeout { timeout: Duration, name: String },
    #[error("could not read {target}: {stderr}")]
    Read { target: String, stderr: String },
    #[error("could not write {target}: {stderr}")]
    Write { target: String, stderr: String },
    #[error("mktemp -d failed: {stderr}")]
    Tempdir { stderr: String },
    #[error("snapshot failed: {stderr}")]
    Snapshot { stderr: String },
    #[error("docker was started without stdout/stderr pipes")]
    Pipes,
}

impl From<DockerSandboxError> for HarnessError {
    fn from(source: DockerSandboxError) -> Self {
        Self::Sandbox {
            source: io::Error::other(source),
        }
    }
}
