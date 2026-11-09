use std::io;
use std::os::unix::process::ExitStatusExt as _;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustix::io::Errno;
use rustix::process::{Pid, Signal, kill_process, kill_process_group};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};
use tokio::process as tokio_process;
use tokio::time::{sleep, timeout};
use tokio_process::{Child, ChildStderr, ChildStdin, ChildStdout};

use super::error::DockerSandboxError;
use crate::errors::{CappedOperation, HarnessError};
use crate::sandbox::{CompletedRun, Process, SandboxCommand};

const KILL_GRACE_SECONDS: u64 = 5;
const WAIT_POLL_MILLIS: u64 = 10;

pub struct DockerProcess {
    pub(super) child: Arc<OwnedChild>,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
    stderr: ChildStderr,
}

pub(super) struct OwnedChild {
    child: Mutex<Child>,
}

impl OwnedChild {
    pub(super) fn signal(&self, signal: Signal) -> Result<(), HarnessError> {
        let mut child = self
            .child
            .lock()
            .map_err(|_poisoned| HarnessError::from(DockerSandboxError::Poisoned))?;
        if child
            .try_wait()
            .map_err(|source| HarnessError::Sandbox { source })?
            .is_some()
        {
            return Ok(());
        }
        let Some(pid) = child
            .id()
            .and_then(|raw| i32::try_from(raw).ok())
            .and_then(Pid::from_raw)
        else {
            return Ok(());
        };
        drop(child);
        match kill_process_group(pid, signal) {
            Ok(()) => Ok(()),
            Err(_group_gone) => match kill_process(pid, signal) {
                Ok(()) | Err(Errno::SRCH) => Ok(()),
                Err(error) => Err(HarnessError::Sandbox {
                    source: io::Error::from(error),
                }),
            },
        }
    }

    pub(super) async fn wait(&self) -> Result<i32, HarnessError> {
        loop {
            let exited = self
                .child
                .lock()
                .map_err(|_poisoned| HarnessError::from(DockerSandboxError::Poisoned))?
                .try_wait()
                .map_err(|source| HarnessError::Sandbox { source })?;
            if let Some(status) = exited {
                return Ok(status
                    .code()
                    .unwrap_or_else(|| status.signal().unwrap_or(1_i32).saturating_neg()));
            }
            sleep(Duration::from_millis(WAIT_POLL_MILLIS)).await;
        }
    }

    pub(super) async fn kill(&self) -> Result<(), HarnessError> {
        drop(self.signal(Signal::TERM));
        match timeout(Duration::from_secs(KILL_GRACE_SECONDS), self.wait()).await {
            Ok(status) => {
                status?;
            }
            Err(_elapsed) => {
                self.signal(Signal::KILL)?;
                self.wait().await?;
            }
        }
        Ok(())
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        drop(self.signal(Signal::KILL));
    }
}

impl Drop for DockerProcess {
    fn drop(&mut self) {
        drop(self.child.signal(Signal::KILL));
    }
}

impl Process for DockerProcess {
    type Stdin = ChildStdin;
    type Stdout = ChildStdout;
    type Stderr = ChildStderr;

    fn stdin(&mut self) -> Option<&mut ChildStdin> {
        self.stdin.as_mut()
    }

    fn close_stdin(&mut self) {
        drop(self.stdin.take());
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

pub(super) fn start(program: &Path, args: &[String]) -> Result<DockerProcess, HarnessError> {
    let mut child = tokio_process::Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| DockerSandboxError::Spawn { source })?;
    let stdin = child.stdin.take();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| HarnessError::from(DockerSandboxError::Pipes))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| HarnessError::from(DockerSandboxError::Pipes))?;
    Ok(DockerProcess {
        child: Arc::new(OwnedChild {
            child: Mutex::new(child),
        }),
        stdin,
        stdout,
        stderr,
    })
}

pub(super) async fn finish(
    process: &mut DockerProcess,
    input: Option<&[u8]>,
    max: usize,
    operation: CappedOperation,
) -> Result<(i32, Vec<u8>, Vec<u8>), HarnessError> {
    if let (Some(stdin), Some(data)) = (process.stdin.as_mut(), input) {
        // a client that exits before reading all of stdin leaves its exit code and stderr as the error
        if let Err(error) = stdin.write_all(data).await
            && error.kind() != io::ErrorKind::BrokenPipe
        {
            return Err(HarnessError::Sandbox { source: error });
        }
    }
    drop(process.stdin.take());
    let (stdout, stderr) = tokio::try_join!(
        read_capped(&mut process.stdout, max, operation),
        read_capped(&mut process.stderr, max, operation)
    )?;
    let exit_code = process.child.wait().await?;
    Ok((exit_code, stdout, stderr))
}

pub(super) async fn collect(
    process: &mut DockerProcess,
    command: &SandboxCommand,
    limit: Duration,
    max: usize,
) -> Result<CompletedRun, HarnessError> {
    let outcome = timeout(
        limit,
        finish(process, None, max, CappedOperation::RunOutput),
    )
    .await;
    let result = match outcome {
        Ok(result) => result,
        Err(_elapsed) => Err(HarnessError::from(DockerSandboxError::Timeout {
            timeout: limit,
            name: command.argv.first().cloned().unwrap_or_default(),
        })),
    };
    if result.is_err() {
        process.kill().await?;
    }
    result.map(|(exit_code, stdout, stderr)| CompletedRun {
        stdout: decode_utf8(&stdout),
        stderr: decode_utf8(&stderr),
        exit_code,
    })
}

pub(super) fn decode_utf8(bytes: &[u8]) -> String {
    let mut decoded = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        decoded.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            decoded.push(char::REPLACEMENT_CHARACTER);
        }
    }
    decoded
}

async fn read_capped<Reader>(
    reader: &mut Reader,
    max: usize,
    operation: CappedOperation,
) -> Result<Vec<u8>, HarnessError>
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
            operation,
            limit: max,
        });
    }
    Ok(data)
}
