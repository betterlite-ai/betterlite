use std::io::{self, Read, Write as _};
use std::num::NonZeroUsize;
use std::panic;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::{self, Scope, ScopedJoinHandle};
use std::time::{Duration, Instant};

use super::SamlIdpError;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60);

// The fixture runs in the pinned oracle environment, with no inherited variables and a deadline.
pub(crate) fn run_fixture_script(script: &str, input: &[u8]) -> Result<Vec<u8>, SamlIdpError> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(SamlIdpError::InputTooLarge);
    }
    let root = Path::new(ROOT).join("../..");
    let python = root.join("tools/oracle/litellm/.venv/bin/python");
    let script_path = root.join("tools/oracle/fixtures/saml-idp").join(script);
    let mut command = fixture_command(&python, &script_path);
    run_bounded(&mut command, input.to_vec(), TIMEOUT)
}

pub(crate) fn fixture_command(python: &Path, script_path: &Path) -> Command {
    let mut command = Command::new("setpriv");
    command
        .args(["--pdeathsig", "KILL", "--"])
        .arg(python)
        .arg(script_path)
        .env_clear()
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONNOUSERSITE", "1")
        .env("LITELLM_LOCAL_MODEL_COST_MAP", "True");
    command
}

// betterlite-oracle-runner's bounded runner is pub(super) in a private module, so this module carries its own.
fn run_bounded(
    command: &mut Command,
    input: Vec<u8>,
    timeout: Duration,
) -> Result<Vec<u8>, SamlIdpError> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(SamlIdpError::Spawn)?;
    thread::scope(|scope| {
        let pumped = pump(scope, &mut child, input);
        let status = wait_for_exit(&mut child, timeout)?;
        let pipes = pumped?;
        let (output, excess) = joined(pipes.output).map_err(SamlIdpError::Io)?;
        let (diagnostic, _diagnostic_excess) =
            joined(pipes.diagnostic).map_err(SamlIdpError::Io)?;
        if excess {
            return Err(SamlIdpError::OutputTooLarge);
        }
        if !status.success() {
            let text = String::from_utf8(diagnostic).map_err(SamlIdpError::Diagnostic)?;
            return Err(SamlIdpError::Exited {
                status,
                diagnostic: text,
            });
        }
        joined(pipes.input).map_err(SamlIdpError::Io)?;
        Ok(output)
    })
}

struct Pipes<'scope> {
    input: ScopedJoinHandle<'scope, io::Result<()>>,
    output: ScopedJoinHandle<'scope, io::Result<(Vec<u8>, bool)>>,
    diagnostic: ScopedJoinHandle<'scope, io::Result<(Vec<u8>, bool)>>,
}

fn pump<'scope>(
    scope: &'scope Scope<'scope, '_>,
    child: &mut Child,
    input: Vec<u8>,
) -> Result<Pipes<'scope>, SamlIdpError> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| SamlIdpError::Pipe(io::Error::other("stdin")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| SamlIdpError::Pipe(io::Error::other("stdout")))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| SamlIdpError::Pipe(io::Error::other("stderr")))?;
    Ok(Pipes {
        input: scope.spawn(move || stdin.write_all(&input)),
        output: scope.spawn(move || read_limited(stdout)),
        diagnostic: scope.spawn(move || read_limited(stderr)),
    })
}

// A pipe thread only panics on a bug, so the panic goes on to the caller as it is.
fn joined<T>(handle: ScopedJoinHandle<'_, T>) -> T {
    handle
        .join()
        .unwrap_or_else(|payload| panic::resume_unwind(payload))
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> Result<ExitStatus, SamlIdpError> {
    let started = Instant::now();
    loop {
        if let Some(exit_status) = child.try_wait().map_err(SamlIdpError::Io)? {
            return Ok(exit_status);
        }
        if started.elapsed() >= timeout {
            child.kill().map_err(SamlIdpError::Io)?;
            child.wait().map_err(SamlIdpError::Io)?;
            return Err(SamlIdpError::Timeout(timeout));
        }
        thread::park_timeout(Duration::from_millis(10));
    }
}

fn read_limited<R>(mut reader: R) -> io::Result<(Vec<u8>, bool)>
where
    R: Read,
{
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut excess = false;
    loop {
        let Some(nonzero_count) = NonZeroUsize::new(reader.read(&mut buffer)?) else {
            return Ok((output, excess));
        };
        let count = nonzero_count.get();
        let remaining = MAX_OUTPUT_BYTES.saturating_sub(output.len());
        let bytes = buffer
            .get(..count.min(remaining))
            .ok_or_else(|| io::Error::other("reader returned an invalid byte count"))?;
        output.extend(bytes.iter().copied());
        excess |= count > remaining;
    }
}
