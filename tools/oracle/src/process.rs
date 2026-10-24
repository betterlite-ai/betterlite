use reqwest::blocking::Client;
use std::fs::File;
use std::io::Error;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::http::{self, HttpError};

pub(crate) struct Gateway {
    pub(crate) child: Child,
    pub(crate) base_url: String,
    pub(crate) log_path: PathBuf,
}

pub(crate) struct GatewaySpec<'a> {
    pub(crate) command: &'a Path,
    pub(crate) args: &'a [String],
    pub(crate) cwd: &'a Path,
    pub(crate) env: &'a [(String, String)],
    pub(crate) port: u16,
    pub(crate) log_path: &'a Path,
}

#[derive(Debug, Error)]
pub(crate) enum ProcessError {
    #[error("cannot open gateway log {path}: {source}")]
    Log { path: PathBuf, source: Error },
    #[error("cannot start gateway: {0}")]
    Spawn(#[source] Error),
    #[error("gateway exited before readiness: {0}")]
    Exited(ExitStatus),
    #[error("gateway did not become ready within 120 seconds")]
    Timeout,
    #[error("gateway request failed while waiting for readiness: {0}")]
    Ready(#[source] HttpError),
    #[error("cannot stop gateway: {0}")]
    Stop(#[source] Error),
}

impl Gateway {
    pub(crate) fn start(spec: &GatewaySpec<'_>) -> Result<Self, ProcessError> {
        let (log, stderr) = File::create(spec.log_path)
            .and_then(|log| Ok((log.try_clone()?, log)))
            .map_err(|source| ProcessError::Log {
                path: spec.log_path.to_owned(),
                source,
            })?;
        let mut process = Command::new(spec.command);
        process
            .args(spec.args)
            .current_dir(spec.cwd)
            .envs(spec.env.iter().cloned())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr));
        let child = process.spawn().map_err(ProcessError::Spawn)?;
        Ok(Self {
            child,
            base_url: format!("http://127.0.0.1:{}", spec.port),
            log_path: spec.log_path.to_owned(),
        })
    }

    pub(crate) fn wait_ready(&mut self, client: &Client) -> Result<(), ProcessError> {
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(120))
            .ok_or(ProcessError::Timeout)?;
        while before_deadline(Instant::now(), deadline) {
            if let Some(status) = self.child.try_wait().map_err(ProcessError::Stop)? {
                return Err(ProcessError::Exited(status));
            }
            match http::ready(client, &self.base_url) {
                Ok(true) => return Ok(()),
                Ok(false) | Err(HttpError::Io(_)) => {}
                Err(HttpError::Request(error)) if error.is_connect() || error.is_timeout() => {}
                Err(error) => return Err(ProcessError::Ready(error)),
            }
            thread::park_timeout(Duration::from_millis(250));
        }
        Err(ProcessError::Timeout)
    }

    pub(crate) fn stop(&mut self) -> Result<(), ProcessError> {
        if self.child.try_wait().map_err(ProcessError::Stop)?.is_none() {
            self.child.kill().map_err(ProcessError::Stop)?;
        }
        self.child.wait().map_err(ProcessError::Stop)?;
        Ok(())
    }
}

pub(crate) fn before_deadline(now: Instant, deadline: Instant) -> bool {
    let before = now < deadline;
    before
}

impl Drop for Gateway {
    fn drop(&mut self) {
        let _stopped = self.stop();
    }
}
