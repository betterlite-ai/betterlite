use std::io::{self, Read, Write as _};
use std::num::NonZeroUsize;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::str;
use std::thread::{self, ScopedJoinHandle};
use std::time::Duration;

use super::capture::Stop;
use super::error::SdkHttpError;

const POLL: Duration = Duration::from_millis(10);
const LOG_LIMIT: usize = 0x0010_0000;

pub(crate) struct Piped {
    pub(crate) stdout: Result<Vec<u8>, SdkHttpError>,
    pub(crate) stderr: Vec<u8>,
}

struct Capped {
    bytes: Vec<u8>,
    excess: bool,
}

type Joined<'scope, T> = ScopedJoinHandle<'scope, io::Result<T>>;

// the child is always killed and waited before this returns, whether it finished, hit the stop or failed
pub(crate) fn run(
    command: &mut Command,
    input: &[u8],
    stdout_limit: usize,
    stop: &Stop<'_>,
) -> Piped {
    let spawned = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(source) => {
            return Piped {
                stdout: Err(SdkHttpError::Spawn(source)),
                stderr: Vec::new(),
            };
        }
    };
    let (Some(mut stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let reaped = reap(&mut child);
        return Piped {
            stdout: reaped.and(Err(SdkHttpError::Io(io::Error::other("missing pipe")))),
            stderr: Vec::new(),
        };
    };
    thread::scope(|scope| {
        let writer = scope.spawn(move || stdin.write_all(input));
        let output = scope.spawn(move || read_capped(stdout, stdout_limit));
        let diagnostics = scope.spawn(move || read_capped(stderr, LOG_LIMIT));
        let waited = wait(&mut child, stop);
        let written = writer.join();
        let diagnostic_bytes = join_capped(diagnostics)
            .map(|capped| capped.bytes)
            .unwrap_or_default();
        Piped {
            stdout: collect(waited, written, output, &diagnostic_bytes, stdout_limit),
            stderr: diagnostic_bytes,
        }
    })
}

fn wait(child: &mut Child, stop: &Stop<'_>) -> Result<ExitStatus, SdkHttpError> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if let Err(stopped) = stop.check() {
                    reap(child)?;
                    return Err(stopped);
                }
            }
            Err(source) => {
                reap(child)?;
                return Err(SdkHttpError::Io(source));
            }
        }
        thread::park_timeout(POLL);
    }
}

fn reap(child: &mut Child) -> Result<(), SdkHttpError> {
    child.kill().map_err(SdkHttpError::Io)?;
    child.wait().map_err(SdkHttpError::Io)?;
    Ok(())
}

fn join_capped(handle: Joined<'_, Capped>) -> Result<Capped, SdkHttpError> {
    handle
        .join()
        .map_err(|_source| SdkHttpError::Join)?
        .map_err(SdkHttpError::Io)
}

fn collect(
    waited: Result<ExitStatus, SdkHttpError>,
    written: thread::Result<io::Result<()>>,
    output: Joined<'_, Capped>,
    diagnostics: &[u8],
    limit: usize,
) -> Result<Vec<u8>, SdkHttpError> {
    let status = waited?;
    let joined = join_capped(output);
    if !status.success() {
        return Err(SdkHttpError::Exited {
            status,
            stderr: str::from_utf8(diagnostics)
                .map_or_else(|_source| "non-UTF-8 output".to_owned(), ToOwned::to_owned),
        });
    }
    let captured = joined?;
    if captured.excess {
        return Err(SdkHttpError::OutputTooLarge { limit });
    }
    written
        .map_err(|_source| SdkHttpError::Join)?
        .map_err(SdkHttpError::Io)?;
    Ok(captured.bytes)
}

fn read_capped<Source>(mut reader: Source, limit: usize) -> io::Result<Capped>
where
    Source: Read,
{
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut excess = false;
    loop {
        let Some(filled) = NonZeroUsize::new(reader.read(&mut buffer)?) else {
            return Ok(Capped { bytes, excess });
        };
        let count = filled.get();
        let room = limit.saturating_sub(bytes.len());
        let kept = buffer
            .get(..count.min(room))
            .ok_or_else(|| io::Error::other("reader returned an invalid byte count"))?;
        bytes.extend_from_slice(kept);
        excess |= count > room;
    }
}
