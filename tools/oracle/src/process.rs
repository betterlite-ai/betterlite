use reqwest::blocking::Client;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::OracleError;
use crate::http;

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

impl Gateway {
    pub(crate) fn start(spec: GatewaySpec) -> Result<Self, OracleError> {
        let (log, stderr) = File::create(&spec.log_path)
            .and_then(|log| Ok((log.try_clone()?, log)))
            .map_err(|source| OracleError::ProcessLog {
                path: spec.log_path.clone(),
                source,
            })?;
        #[cfg(unix)]
        let mut process = {
            let mut process = Command::new("setpriv");
            process
                .args(["--pdeathsig", "KILL", "--"])
                .arg(&spec.command);
            process
        };
        #[cfg(not(unix))]
        let mut process = Command::new(&spec.command);
        process
            .args(spec.args)
            .current_dir(spec.cwd)
            .envs(spec.env)
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr));
        let child = process.spawn().map_err(OracleError::ProcessSpawn)?;
        Ok(Self {
            child,
            base_url: format!("http://127.0.0.1:{}", spec.port),
            log_path: spec.log_path,
        })
    }

    pub(crate) fn wait_ready(&mut self, client: &Client) -> Result<(), OracleError> {
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(120))
            .ok_or(OracleError::ProcessTimeout)?;
        while Instant::now().checked_duration_since(deadline).is_none() {
            if let Some(status) = self.child.try_wait().map_err(OracleError::ProcessStop)? {
                return Err(OracleError::ProcessExited(status));
            }
            match http::ready(client, &self.base_url) {
                Ok(true) => return Ok(()),
                Ok(false) | Err(OracleError::HttpIo(_)) => {}
                Err(OracleError::HttpRequest(error))
                    if error.is_connect() || error.is_timeout() => {}
                Err(error) => {
                    return Err(OracleError::ProcessReady {
                        source: Box::new(error),
                    });
                }
            }
            thread::park_timeout(Duration::from_millis(250));
        }
        Err(OracleError::ProcessTimeout)
    }

    pub(crate) fn stop(&mut self) -> Result<(), OracleError> {
        if self
            .child
            .try_wait()
            .map_err(OracleError::ProcessStop)?
            .is_none()
        {
            self.child.kill().map_err(OracleError::ProcessStop)?;
        }
        self.child.wait().map_err(OracleError::ProcessStop)?;
        Ok(())
    }
}

pub(crate) fn sync_litellm(root: &Path) -> Result<(), OracleError> {
    let project = root.join("tools/oracle/litellm");
    let status = Command::new("uv")
        .args(["sync", "--locked"])
        .current_dir(project)
        .status()
        .map_err(OracleError::ProcessSync)?;
    if status.success() {
        Ok(())
    } else {
        Err(OracleError::ProcessSyncExited(status))
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        let _stopped = self.stop();
    }
}
