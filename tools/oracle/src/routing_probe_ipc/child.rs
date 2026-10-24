use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::io::{self, AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};
use tokio::process::{Child, Command};
use tokio::time;

use super::MAX_MESSAGE_BYTES;
use crate::OracleError;

// one byte past the limit tells a full message from an oversized one
const READ_LIMIT: u64 = (1 << 20) + 1;

#[derive(Debug, Clone)]
pub struct ProbeCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
}

#[derive(Debug)]
pub struct Captured {
    pub bytes: Vec<u8>,
    pub truncated: bool,
}

#[derive(Debug)]
pub struct ChildOutput {
    pub status: ExitStatus,
    pub stdout: Captured,
    pub stderr: Captured,
}

pub async fn run_bounded(
    command: &ProbeCommand,
    input: &[u8],
    deadline: Duration,
) -> Result<ChildOutput, OracleError> {
    if input.len() > MAX_MESSAGE_BYTES {
        return Err(OracleError::ProbeTooLarge);
    }
    // setpriv execs the program in place, so kill_on_drop and the death signal reach the real probe
    let mut child = Command::new("setpriv")
        .args(["--pdeathsig", "KILL", "--"])
        .arg(&command.program)
        .args(&command.args)
        .current_dir(&command.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(OracleError::ProbeSpawn)?;
    time::timeout(deadline, exchange(&mut child, input))
        .await
        .map_err(|_elapsed| OracleError::ProbeTimeout(deadline))?
}

async fn exchange(child: &mut Child, input: &[u8]) -> Result<ChildOutput, OracleError> {
    let (Some(mut request_pipe), Some(reply_pipe), Some(log_pipe)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err(OracleError::ProbePipe);
    };
    let writing = async move { request_pipe.write_all(input).await };
    let (written, reply, log, status) = tokio::join!(
        writing,
        capture(reply_pipe),
        capture(log_pipe),
        child.wait()
    );
    written.map_err(OracleError::ProbeIo)?;
    Ok(ChildOutput {
        status: status.map_err(OracleError::ProbeIo)?,
        stdout: reply.map_err(OracleError::ProbeIo)?,
        stderr: log.map_err(OracleError::ProbeIo)?,
    })
}

async fn capture<Stream>(stream: Stream) -> io::Result<Captured>
where
    Stream: AsyncRead + Unpin,
{
    let mut limited = stream.take(READ_LIMIT);
    let mut bytes = Vec::new();
    limited.read_to_end(&mut bytes).await?;
    let truncated = bytes.len() > MAX_MESSAGE_BYTES;
    bytes.truncate(MAX_MESSAGE_BYTES);
    // the rest is drained so a chatty child never blocks on a full pipe
    io::copy(&mut limited.into_inner(), &mut io::sink()).await?;
    Ok(Captured { bytes, truncated })
}
