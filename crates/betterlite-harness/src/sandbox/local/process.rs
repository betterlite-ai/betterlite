use std::collections::BTreeMap;
use std::io;
use std::os::unix::process::ExitStatusExt as _;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustix::io::Errno;
use rustix::process::{Pid, Signal, kill_process, kill_process_group};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncRead, AsyncReadExt as _};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::time::{sleep, timeout};

use super::error::SandboxError;
use super::files::decode_utf8;
use crate::errors::{CappedOperation, HarnessError};
use crate::sandbox::{CompletedRun, Process, SandboxCommand};

const PROCESS_KILL_GRACE_SECONDS: u64 = 5;

pub(super) struct OwnedChild {
    child: Mutex<Child>,
}

impl OwnedChild {
    pub(super) fn signal(&self, signal: Signal) -> Result<(), HarnessError> {
        let mut child = self
            .child
            .lock()
            .map_err(|_poisoned| SandboxError::Poisoned)?;
        if child
            .try_wait()
            .map_err(|source| HarnessError::Sandbox { source })?
            .is_some()
        {
            return Ok(());
        }
        if let Some(pid) = child
            .id()
            .and_then(|raw| i32::try_from(raw).ok())
            .and_then(Pid::from_raw)
            && kill_process_group(pid, signal).is_err()
            && let Err(error) = kill_process(pid, signal)
            && error != Errno::SRCH
        {
            return Err(HarnessError::Sandbox {
                source: io::Error::from(error),
            });
        }
        drop(child);
        Ok(())
    }

    pub(super) async fn wait(&self) -> Result<i32, HarnessError> {
        loop {
            let status = self
                .child
                .lock()
                .map_err(|_poisoned| SandboxError::Poisoned)?
                .try_wait()
                .map_err(|source| HarnessError::Sandbox { source })?;
            if let Some(exit) = status {
                return Ok(exit
                    .code()
                    .unwrap_or_else(|| exit.signal().unwrap_or(1_i32).saturating_neg()));
            }
            sleep(Duration::from_millis(10)).await;
        }
    }

    pub(super) async fn kill(&self) -> Result<(), HarnessError> {
        drop(self.signal(Signal::TERM));
        if let Ok(result) =
            timeout(Duration::from_secs(PROCESS_KILL_GRACE_SECONDS), self.wait()).await
        {
            result?;
        } else {
            self.signal(Signal::KILL)?;
            self.wait().await?;
        }
        Ok(())
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        drop(self.signal(Signal::KILL));
    }
}

pub struct SubprocessHandle {
    pub(super) child: Arc<OwnedChild>,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
    stderr: ChildStderr,
}

impl Process for SubprocessHandle {
    type Stdin = ChildStdin;
    type Stdout = ChildStdout;
    type Stderr = ChildStderr;

    fn stdin(&mut self) -> Option<&mut ChildStdin> {
        self.stdin.as_mut()
    }
    fn stdout(&mut self) -> &mut ChildStdout {
        &mut self.stdout
    }
    fn stderr(&mut self) -> &mut ChildStderr {
        &mut self.stderr
    }
    async fn wait(&mut self) -> Result<i32, HarnessError> {
        self.child.wait().await
    }
    async fn kill(&mut self) -> Result<(), HarnessError> {
        self.child.kill().await
    }
}

pub(super) fn start(
    command: &SandboxCommand,
    cwd: &str,
    environment: &BTreeMap<String, SecretString>,
) -> Result<SubprocessHandle, HarnessError> {
    let name = command.argv.first().ok_or(SandboxError::EmptyCommand)?;
    let mut child = Command::new(name)
        .args(command.argv.iter().skip(1))
        .current_dir(cwd)
        .env_clear()
        .envs(
            environment
                .iter()
                .map(|(key, value)| (key, value.expose_secret())),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| SandboxError::Start {
            name: name.clone(),
            source,
        })?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take().ok_or(SandboxError::Pipes)?;
    let stderr = child.stderr.take().ok_or(SandboxError::Pipes)?;
    Ok(SubprocessHandle {
        child: Arc::new(OwnedChild {
            child: Mutex::new(child),
        }),
        stdin,
        stdout,
        stderr,
    })
}

pub(super) async fn collect(
    process: &mut SubprocessHandle,
    command: &SandboxCommand,
    limit: Duration,
    max: usize,
) -> Result<CompletedRun, HarnessError> {
    drop(process.stdin.take());
    let output = async {
        let (stdout, stderr) = tokio::try_join!(
            read_capped(&mut process.stdout, max),
            read_capped(&mut process.stderr, max)
        )?;
        let exit_code = process.wait().await?;
        Ok(CompletedRun {
            stdout: decode_utf8(&stdout),
            stderr: decode_utf8(&stderr),
            exit_code,
        })
    };
    let result = timeout(limit, output).await.unwrap_or_else(|_elapsed| {
        Err(HarnessError::from(SandboxError::Timeout {
            timeout: limit,
            name: command.argv.first().cloned().unwrap_or_default(),
        }))
    });
    if result.is_err() {
        process.kill().await?;
    }
    result
}

async fn read_capped<Reader>(reader: &mut Reader, max: usize) -> Result<Vec<u8>, HarnessError>
where
    Reader: AsyncRead + Unpin,
{
    let mut data = Vec::new();
    reader
        .take(u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut data)
        .await
        .map_err(|source| HarnessError::Sandbox { source })?;
    if data.len() > max {
        return Err(HarnessError::ContentLimitExceeded {
            operation: CappedOperation::RunOutput,
            limit: max,
        });
    }
    Ok(data)
}
