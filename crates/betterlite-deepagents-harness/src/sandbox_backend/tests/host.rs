use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::path::Path;
use std::time::Duration;

use betterlite_harness::errors::HarnessError;
use betterlite_harness::sandbox::{CompletedRun, Process, Sandbox, SandboxCommand};
use tokio::fs;
use tokio::io::{Empty, Sink};
use tokio::process::Command;
use tokio::time;

// runs every command on the host inside workdir, the way LocalSandbox does
pub(super) struct HostSandbox {
    pub(super) workdir: String,
    pub(super) fail_find: bool,
}

pub(super) struct NoProcess {
    stdout: Empty,
    stderr: Empty,
}

fn sandbox_error(source: io::Error) -> HarnessError {
    HarnessError::Sandbox { source }
}

impl Process for NoProcess {
    type Stdin = Sink;
    type Stdout = Empty;
    type Stderr = Empty;

    fn stdin(&mut self) -> Option<&mut Sink> {
        None
    }

    fn stdout(&mut self) -> &mut Empty {
        &mut self.stdout
    }

    fn stderr(&mut self) -> &mut Empty {
        &mut self.stderr
    }

    fn wait(&mut self) -> impl Future<Output = Result<i32, HarnessError>> + Send {
        ready(Ok(0_i32))
    }

    fn kill(&mut self) -> impl Future<Output = Result<(), HarnessError>> + Send {
        ready(Ok(()))
    }
}

impl Sandbox for HostSandbox {
    type Process = NoProcess;

    fn workdir(&self) -> &str {
        &self.workdir
    }

    fn exec(
        &self,
        _command: &SandboxCommand,
    ) -> impl Future<Output = Result<NoProcess, HarnessError>> + Send {
        ready(Err(HarnessError::SessionClosed))
    }

    async fn run(
        &self,
        command: &SandboxCommand,
        timeout: Duration,
        _max_output_bytes: usize,
    ) -> Result<CompletedRun, HarnessError> {
        if self.fail_find
            && command
                .argv
                .first()
                .is_some_and(|program| program == "find")
        {
            return Ok(CompletedRun {
                stdout: format!("{}/README.md\n", self.workdir),
                stderr: "find: simulated traversal failure\n".to_owned(),
                exit_code: 1,
            });
        }
        let (program, args) = command.argv.split_first().unwrap();
        let mut child = Command::new("setpriv");
        child
            .arg("--pdeathsig")
            .arg("KILL")
            .arg("--")
            .arg(program)
            .args(args)
            .current_dir(&self.workdir)
            .kill_on_drop(true);
        let output = time::timeout(timeout, child.output())
            .await
            .map_err(|elapsed| sandbox_error(io::Error::new(io::ErrorKind::TimedOut, elapsed)))?
            .map_err(sandbox_error)?;
        Ok(CompletedRun {
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
            exit_code: output.status.code().unwrap(),
        })
    }

    async fn read(&self, path: &str, _max_bytes: usize) -> Result<Vec<u8>, HarnessError> {
        fs::read(path).await.map_err(sandbox_error)
    }

    async fn write(&self, path: &str, data: &[u8]) -> Result<(), HarnessError> {
        let parent = Path::new(path).parent().unwrap();
        fs::create_dir_all(parent).await.map_err(sandbox_error)?;
        fs::write(path, data).await.map_err(sandbox_error)
    }

    fn host_url(&self, port: u16) -> String {
        format!("http://127.0.0.1:{port}")
    }

    fn which(
        &self,
        _binary: &str,
    ) -> impl Future<Output = Result<Option<String>, HarnessError>> + Send {
        ready(Ok(None))
    }

    fn snapshot(
        &self,
    ) -> impl Future<Output = Result<BTreeMap<String, String>, HarnessError>> + Send {
        ready(Ok(BTreeMap::new()))
    }

    fn tempdir(&self) -> impl Future<Output = Result<String, HarnessError>> + Send {
        ready(Ok(self.workdir.clone()))
    }

    fn close(&self) -> impl Future<Output = Result<(), HarnessError>> + Send {
        ready(Ok(()))
    }
}
