use std::collections::VecDeque;
use std::io;

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};

use crate::errors::HarnessError;
use crate::sandbox::Process;

pub const HARNESS_STDERR_TAIL_LINES: usize = 40;
pub const HARNESS_STREAM_CHUNK_BYTES: usize = 64 * 1024;

// splits off one complete line (without its newline) once `buffer` holds one
pub(super) fn take_line(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let index = buffer.iter().position(|byte| *byte == b'\n')?;
    let mut line = buffer.drain(..=index).collect::<Vec<_>>();
    line.pop();
    Some(line)
}

pub(super) fn push_tail(tail: &mut VecDeque<String>, line: String) {
    if tail.len() == HARNESS_STDERR_TAIL_LINES {
        tail.pop_front();
    }
    tail.push_back(line);
}

pub async fn iter_stream_lines<Reader>(mut stream: Reader) -> Result<Vec<Vec<u8>>, HarnessError>
where
    Reader: AsyncRead + Unpin,
{
    let mut buffer = Vec::new();
    let mut chunk = vec![0_u8; HARNESS_STREAM_CHUNK_BYTES];
    let mut lines = Vec::new();
    loop {
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|source| HarnessError::Sandbox { source })?;
        if read == 0 {
            break;
        }
        let Some(filled) = chunk.get(..read) else {
            return Err(HarnessError::Sandbox {
                source: io::Error::other("stream read exceeded its buffer"),
            });
        };
        buffer.extend_from_slice(filled);
        while let Some(line) = take_line(&mut buffer) {
            lines.push(line);
        }
    }
    if !buffer.is_empty() {
        lines.push(buffer);
    }
    Ok(lines)
}

// diagnostic text only, so invalid UTF-8 is replaced rather than rejected, matching LiteLLM
pub(super) fn decode_lossy(bytes: &[u8]) -> String {
    let mut text = String::new();
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push('\u{fffd}');
        }
    }
    text
}

pub async fn drain_stderr<Reader>(
    stream: Reader,
    tail: &mut VecDeque<String>,
) -> Result<(), HarnessError>
where
    Reader: AsyncRead + Unpin,
{
    for line in iter_stream_lines(stream).await? {
        push_tail(tail, decode_lossy(&line));
    }
    Ok(())
}

pub async fn send_stdin<Proc>(process: &mut Proc, data: &str) -> Result<(), HarnessError>
where
    Proc: Process,
{
    let stdin = process.stdin().ok_or_else(|| HarnessError::Sandbox {
        source: io::Error::new(io::ErrorKind::BrokenPipe, "harness process has no stdin"),
    })?;
    stdin
        .write_all(data.as_bytes())
        .await
        .map_err(|source| HarnessError::Sandbox { source })?;
    stdin
        .shutdown()
        .await
        .map_err(|source| HarnessError::Sandbox { source })
}
