use std::future::{Future, ready};
use std::io;
use std::process::Stdio;
use std::sync::Arc;

use rustix::io::Errno;
use rustix::process::{Pid, Signal, kill_process_group};
use tokio::process as tokio_process;
use tokio_process::{Child, ChildStdin, ChildStdout};

use super::error::ExitFailure;
use super::transport::{StdioServerParameters, TransportContext};

pub struct StdioStreams {
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
}

// The server leads its own process group, so one kill reaches it and anything it started
pub struct StdioTransport {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
}

impl StdioTransport {
    pub fn spawn(parameters: StdioServerParameters) -> io::Result<(Self, u32)> {
        let StdioServerParameters { command, args, env } = parameters;
        let mut child = tokio_process::Command::new(command)
            .args(args)
            .env_clear()
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()?;
        let pid = child
            .id()
            .ok_or_else(|| io::Error::other("the stdio server has no process id"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("the stdio server has no stdin pipe"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("the stdio server has no stdout pipe"))?;
        let transport = Self {
            child: Some(child),
            stdin: Some(stdin),
            stdout: Some(stdout),
        };
        Ok((transport, pid))
    }
}

impl TransportContext for StdioTransport {
    type Streams = StdioStreams;
    type Error = io::Error;

    fn enter(&mut self) -> impl Future<Output = Result<Self::Streams, io::Error>> + Send {
        let opened = self.stdin.take().zip(self.stdout.take());
        let pipes = opened.ok_or_else(|| io::Error::other("the stdio server is already open"));
        ready(pipes.map(|(stdin, stdout)| StdioStreams { stdin, stdout }))
    }

    fn exit(mut self) -> impl Future<Output = Result<(), ExitFailure<io::Error>>> + Send + 'static {
        let leader = self.child.as_ref().and_then(Child::id);
        async move {
            kill_group(leader)?;
            if let Some(mut child) = self.child.take() {
                child.wait().await.map_err(ExitFailure::from)?;
            }
            Ok(())
        }
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_ref() {
            drop(kill_group(child.id()));
        }
    }
}

fn kill_group(pid: Option<u32>) -> Result<(), ExitFailure<io::Error>> {
    let Some(leader) = pid
        .and_then(|raw| i32::try_from(raw).ok())
        .and_then(Pid::from_raw)
    else {
        return Ok(());
    };
    match kill_process_group(leader, Signal::KILL) {
        Ok(()) | Err(Errno::SRCH) => Ok(()),
        Err(error) => Err(ExitFailure::Failure(Arc::new(io::Error::from(error)))),
    }
}
