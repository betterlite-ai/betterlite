use std::fs;
use std::path::{Path, PathBuf};
use std::process as std_process;
use std::process::Stdio;

#[cfg(all(unix, not(target_os = "linux")))]
use std::os::unix::process::CommandExt as _;

#[cfg(all(unix, not(target_os = "linux")))]
use rustix::process::{Pid, Signal, kill_process_group};

use super::error::ProcessError;

pub struct LaunchRequest<'request> {
    pub executable: &'request Path,
    pub arguments: &'request [String],
    pub log_path: &'request Path,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitCode {
    pub code: Option<i32>,
}

pub trait LaunchedProcess {
    fn pid(&self) -> u32;

    fn has_exited(&mut self) -> Result<Option<ExitCode>, ProcessError>;
}

pub trait ProcessLauncher {
    fn launch(&self, request: &LaunchRequest<'_>)
    -> Result<Box<dyn LaunchedProcess>, ProcessError>;
}

pub struct GatewayLauncher;

impl ProcessLauncher for GatewayLauncher {
    fn launch(
        &self,
        request: &LaunchRequest<'_>,
    ) -> Result<Box<dyn LaunchedProcess>, ProcessError> {
        if let Some(directory) = request.log_path.parent() {
            fs::create_dir_all(directory).map_err(|source| ProcessError::LogFile {
                path: request.log_path.to_owned(),
                source,
            })?;
        }
        let log = fs::File::create(request.log_path).map_err(|source| ProcessError::LogFile {
            path: request.log_path.to_owned(),
            source,
        })?;
        let errors = log.try_clone().map_err(|source| ProcessError::LogFile {
            path: request.log_path.to_owned(),
            source,
        })?;
        let mut command = gateway_command(request);
        command.stdout(Stdio::from(log)).stderr(Stdio::from(errors));
        let program = PathBuf::from(command.get_program());
        let child = command
            .spawn()
            .map_err(|source| ProcessError::Launch { program, source })?;
        Ok(Box::new(ChildProcess { child }))
    }
}

#[must_use]
pub fn gateway_command(request: &LaunchRequest<'_>) -> std_process::Command {
    #[cfg(target_os = "linux")]
    let mut command = {
        // setpriv execs the gateway in place, so the death signal reaches the gateway itself
        let mut command = std_process::Command::new("setpriv");
        command
            .args(["--pdeathsig", "KILL", "--"])
            .arg(request.executable);
        command
    };
    #[cfg(all(unix, not(target_os = "linux")))]
    let mut command = {
        let mut command = std_process::Command::new(request.executable);
        // setpriv is a util-linux tool, so other Unix systems lead their own process group instead
        let _grouped = command.process_group(0);
        command
    };
    #[cfg(not(unix))]
    let mut command = std_process::Command::new(request.executable);
    command.args(request.arguments);
    command
}

struct ChildProcess {
    child: std_process::Child,
}

impl LaunchedProcess for ChildProcess {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn has_exited(&mut self) -> Result<Option<ExitCode>, ProcessError> {
        let exited = self
            .child
            .try_wait()
            .map_err(|source| ProcessError::ProcessStatus { source })?;
        Ok(exited.map(|status| ExitCode {
            code: status.code(),
        }))
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        // The handle owns the child, so dropping it must not leave a gateway nobody can stop
        #[cfg(all(unix, not(target_os = "linux")))]
        {
            let leader = i32::try_from(self.child.id()).ok().and_then(Pid::from_raw);
            if let Some(leader) = leader {
                let _ignored = kill_process_group(leader, Signal::KILL);
            }
        }
        let _killed = self.child.kill();
        let _reaped = self.child.wait();
    }
}
