use reqwest::blocking::Client;
use std::fs::File;
use std::io::Error;
use std::path::PathBuf;
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

pub(crate) struct GatewaySpec {
    pub(crate) command: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: PathBuf,
    pub(crate) env: Vec<(String, String)>,
    pub(crate) port: u16,
    pub(crate) log_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum ProcessError {
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
    pub(crate) fn start(spec: GatewaySpec) -> Result<Self, ProcessError> {
        let (log, stderr) = File::create(&spec.log_path)
            .and_then(|log| Ok((log.try_clone()?, log)))
            .map_err(|source| ProcessError::Log {
                path: spec.log_path.clone(),
                source,
            })?;
        let mut process = Command::new(spec.command);
        process
            .args(spec.args)
            .current_dir(spec.cwd)
            .envs(spec.env)
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr));
        let child = process.spawn().map_err(ProcessError::Spawn)?;
        Ok(Self {
            child,
            base_url: format!("http://127.0.0.1:{}", spec.port),
            log_path: spec.log_path,
        })
    }

    pub(crate) fn wait_ready(&mut self, client: &Client) -> Result<(), ProcessError> {
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(120))
            .ok_or(ProcessError::Timeout)?;
        while Instant::now().checked_duration_since(deadline).is_none() {
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

impl Drop for Gateway {
    fn drop(&mut self) {
        let _stopped = self.stop();
    }
}
