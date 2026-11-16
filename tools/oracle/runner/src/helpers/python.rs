use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::io::{self, Read, Write as _};
use std::num::NonZeroUsize;
use std::panic;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::{self, Scope, ScopedJoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use super::HelperCall;
use crate::error::HelperRunError;

const PASSED: [&str; 4] = ["PATH", "HOME", "UV_CACHE_DIR", "UV_PYTHON_CACHE_DIR"];

#[derive(Serialize)]
struct Request<'call> {
    operation: &'call str,
    input: &'call Value,
}

#[expect(
    clippy::disallowed_methods,
    reason = "the oracle reads its cache mode and the variables it hands the Python helper from the process environment through this one function"
)]
pub(super) fn process_environment() -> BTreeMap<&'static str, OsString> {
    PASSED
        .into_iter()
        .chain(["BETTERLITE_ORACLE_CACHE"])
        .filter_map(|name| env::var_os(name).map(|value| (name, value)))
        .collect()
}

pub(super) fn python_stdout(
    root: &Path,
    call: &HelperCall,
    case_env: &BTreeMap<String, String>,
    process_env: &BTreeMap<&'static str, OsString>,
) -> Result<Vec<u8>, HelperRunError> {
    let mut command = Command::new("setpriv");
    command
        .args(["--pdeathsig", "KILL", "--"])
        .arg(root.join("tools/oracle/litellm/.venv/bin/python"))
        .arg(root.join("tools/oracle/helpers/dispatch.py"))
        .env_clear();
    configure_environment(&mut command, process_env, case_env);
    let request = Request {
        operation: &call.operation,
        input: &call.input,
    };
    let bytes = serde_json::to_vec(&request).map_err(HelperRunError::Json)?;
    run_bounded(&mut command, bytes, Duration::from_secs(60))
}

pub(super) fn run_bounded(
    command: &mut Command,
    input: Vec<u8>,
    timeout: Duration,
) -> Result<Vec<u8>, HelperRunError> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(HelperRunError::HelperSpawn)?;
    thread::scope(|scope| {
        let pumped = pump_helper_pipes(scope, &mut child, input);
        let status = wait_for_helper_exit(&mut child, timeout)?;
        let pipes = pumped?;
        let (stdout_output, stdout_excess) =
            joined(pipes.output).map_err(HelperRunError::HelperIo)?;
        let (stderr_output, _stderr_excess) =
            joined(pipes.diagnostic).map_err(HelperRunError::HelperIo)?;
        if stdout_excess {
            return Err(HelperRunError::HelperOutputTooLarge);
        }
        if !status.success() {
            let diagnostic = String::from_utf8(stderr_output)
                .map_err(|source| HelperRunError::HelperDiagnostic { status, source })?;
            return Err(HelperRunError::HelperExited { status, diagnostic });
        }
        joined(pipes.input).map_err(HelperRunError::HelperIo)?;
        Ok(stdout_output)
    })
}

struct HelperPipes<'scope> {
    input: ScopedJoinHandle<'scope, io::Result<()>>,
    output: ScopedJoinHandle<'scope, io::Result<(Vec<u8>, bool)>>,
    diagnostic: ScopedJoinHandle<'scope, io::Result<(Vec<u8>, bool)>>,
}

fn pump_helper_pipes<'scope>(
    scope: &'scope Scope<'scope, '_>,
    child: &mut Child,
    input: Vec<u8>,
) -> Result<HelperPipes<'scope>, HelperRunError> {
    let mut stdin_pipe = child.stdin.take().ok_or(HelperRunError::HelperStdin)?;
    let stdout_pipe = child.stdout.take().ok_or(HelperRunError::HelperStdout)?;
    let stderr_pipe = child.stderr.take().ok_or(HelperRunError::HelperStderr)?;
    Ok(HelperPipes {
        input: scope.spawn(move || stdin_pipe.write_all(&input)),
        output: scope.spawn(move || read_limited(stdout_pipe)),
        diagnostic: scope.spawn(move || read_limited(stderr_pipe)),
    })
}

// a pipe thread only panics on a bug, so the panic goes on to the caller as it is
fn joined<T>(handle: ScopedJoinHandle<'_, T>) -> T {
    handle
        .join()
        .unwrap_or_else(|payload| panic::resume_unwind(payload))
}

fn wait_for_helper_exit(
    child: &mut Child,
    timeout: Duration,
) -> Result<ExitStatus, HelperRunError> {
    let started = Instant::now();
    loop {
        if let Some(exit_status) = child.try_wait().map_err(HelperRunError::HelperIo)? {
            return Ok(exit_status);
        }
        if started.elapsed() >= timeout {
            child.kill().map_err(HelperRunError::HelperIo)?;
            child.wait().map_err(HelperRunError::HelperIo)?;
            return Err(HelperRunError::HelperTimeout);
        }
        thread::park_timeout(Duration::from_millis(10));
    }
}

pub(super) fn configure_environment(
    command: &mut Command,
    process_env: &BTreeMap<&'static str, OsString>,
    case_env: &BTreeMap<String, String>,
) {
    for name in PASSED {
        if let Some(value) = process_env.get(name) {
            command.env(name, value);
        }
    }
    for (name, value) in case_env {
        command.env(name, value);
    }
    command.env("LITELLM_LOCAL_MODEL_COST_MAP", "True");
}

pub(super) fn read_limited<R>(mut reader: R) -> io::Result<(Vec<u8>, bool)>
where
    R: Read,
{
    const MAX_OUTPUT: usize = 1024 * 1024;
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut excess = false;
    loop {
        let Some(nonzero_count) = NonZeroUsize::new(reader.read(&mut buffer)?) else {
            return Ok((output, excess));
        };
        let count = nonzero_count.get();
        let remaining = MAX_OUTPUT.saturating_sub(output.len());
        let bytes = buffer
            .get(..count.min(remaining))
            .ok_or_else(|| io::Error::other("reader returned an invalid byte count"))?;
        output.extend(bytes.iter().copied());
        excess |= count > remaining;
    }
}
