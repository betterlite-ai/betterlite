use std::future::{Future, ready};
use std::io;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use rustix::io::Errno;
use rustix::process::{Pid, Signal, kill_process_group};
use tokio::process as tokio_process;
use tokio::time::timeout;
use tokio_process::{Child, ChildStdin, ChildStdout};

use super::error::ExitFailure;
use super::transport::{StdioServerParameters, TransportContext};

pub const PROCESS_TERMINATION_TIMEOUT: Duration = Duration::from_secs(2);

pub struct StdioStreams {
    pub pid: u32,
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
}

pub struct StdioServer {
    pub(super) parameters: StdioServerParameters,
    pub(super) child: Option<Child>,
    pub(super) stdin: Option<ChildStdin>,
    pub(super) stdout: Option<ChildStdout>,
}

// The server leads its own process group, so one kill reaches it and anything it started
pub enum StdioTransport {
    Server(Box<StdioServer>),
    Unsupported(&'static str),
}

impl StdioServer {
    pub(super) fn start(&mut self) -> io::Result<()> {
        if self.child.is_some() {
            return Err(io::Error::other("the stdio server is already open"));
        }
        let mut environment = self.parameters.defaults.clone();
        environment.extend(self.parameters.env.clone());
        let mut child = tokio_process::Command::new(&self.parameters.command)
            .args(&self.parameters.args)
            .env_clear()
            .envs(environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()?;
        self.stdin = child.stdin.take();
        self.stdout = child.stdout.take();
        self.child = Some(child);
        Ok(())
    }

    fn take_streams(&mut self) -> io::Result<StdioStreams> {
        let Some(child) = self.child.as_ref() else {
            return Err(io::Error::other("the stdio server is not open"));
        };
        let pid = child
            .id()
            .ok_or_else(|| io::Error::other("the stdio server has no process id"))?;
        let stdin = self
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("the stdio server has no stdin pipe"))?;
        let stdout = self
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("the stdio server has no stdout pipe"))?;
        Ok(StdioStreams { pid, stdin, stdout })
    }

    async fn shutdown(mut self) -> Result<(), ExitFailure<io::Error>> {
        drop(self.stdin.take());
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        let leader = child.id();
        if timeout(PROCESS_TERMINATION_TIMEOUT, child.wait())
            .await
            .is_err()
        {
            kill_group(leader)?;
            child.wait().await.map_err(ExitFailure::from)?;
        }
        Ok(())
    }
}

impl Drop for StdioServer {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_ref() {
            drop(kill_group(child.id()));
        }
    }
}

impl TransportContext for StdioTransport {
    type Streams = StdioStreams;
    type Error = io::Error;

    fn enter(&mut self) -> impl Future<Output = Result<Self::Streams, io::Error>> + Send {
        let opened = match self {
            Self::Server(server) => server.start().and_then(|()| server.take_streams()),
            Self::Unsupported(name) => Err(unsupported(name)),
        };
        ready(opened)
    }

    fn exit(self) -> impl Future<Output = Result<(), ExitFailure<io::Error>>> + Send + 'static {
        let transport = self;
        async move {
            match transport {
                Self::Server(server) => server.shutdown().await,
                Self::Unsupported(_) => Ok(()),
            }
        }
    }
}

fn unsupported(name: &'static str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("the stdio runtime has no {name} transport"),
    )
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
