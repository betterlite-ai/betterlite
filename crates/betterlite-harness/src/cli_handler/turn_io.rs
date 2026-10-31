use std::future::poll_fn;
use std::io;
use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, ReadBuf};

use super::io::{HARNESS_STREAM_CHUNK_BYTES, take_line};
use crate::errors::HarnessError;
use crate::sandbox::Process;

enum ChunkPoll {
    Pending,
    Eof,
    Data(Vec<u8>),
}

fn poll_chunk<R>(
    stream: &mut R,
    cx: &mut Context<'_>,
    scratch: &mut [u8],
) -> Result<ChunkPoll, io::Error>
where
    R: AsyncRead + Unpin,
{
    let mut buf = ReadBuf::new(scratch);
    match Pin::new(stream).poll_read(cx, &mut buf) {
        Poll::Ready(Ok(())) if buf.filled().is_empty() => Ok(ChunkPoll::Eof),
        Poll::Ready(Ok(())) => Ok(ChunkPoll::Data(buf.filled().to_vec())),
        Poll::Ready(Err(error)) => Err(error),
        Poll::Pending => Ok(ChunkPoll::Pending),
    }
}

// which of stdout/stderr has already reached EOF, so `read_step` knows which pipe to skip
pub(super) struct PipeDone {
    pub(super) stdout: bool,
    pub(super) stderr: bool,
}

// polls whichever stream is not yet done, so a full pipe on one side never blocks the other
async fn read_step<Proc>(
    proc: &mut Proc,
    stdout_scratch: &mut [u8],
    stderr_scratch: &mut [u8],
    done: &PipeDone,
) -> Result<(ChunkPoll, ChunkPoll), HarnessError>
where
    Proc: Process,
{
    let mut poll_both = move |cx: &mut Context<'_>| {
        let stdout = if done.stdout {
            ChunkPoll::Pending
        } else {
            match poll_chunk(proc.stdout(), cx, stdout_scratch) {
                Ok(outcome) => outcome,
                Err(source) => return Poll::Ready(Err(HarnessError::Sandbox { source })),
            }
        };
        let stderr = if done.stderr {
            ChunkPoll::Pending
        } else {
            match poll_chunk(proc.stderr(), cx, stderr_scratch) {
                Ok(outcome) => outcome,
                Err(source) => return Poll::Ready(Err(HarnessError::Sandbox { source })),
            }
        };
        if matches!(stdout, ChunkPoll::Pending) && matches!(stderr, ChunkPoll::Pending) {
            Poll::Pending
        } else {
            Poll::Ready(Ok((stdout, stderr)))
        }
    };
    poll_fn(&mut poll_both).await
}

fn apply_chunk(outcome: ChunkPoll, buffer: &mut Vec<u8>, done: &mut bool) {
    match outcome {
        ChunkPoll::Data(data) => buffer.extend_from_slice(&data),
        ChunkPoll::Eof => *done = true,
        ChunkPoll::Pending => {}
    }
}

pub(super) struct TurnCursor {
    stdout_buf: Vec<u8>,
    stderr_buf: Vec<u8>,
    stdout_scratch: Vec<u8>,
    stderr_scratch: Vec<u8>,
    stdout_done: bool,
    stderr_done: bool,
}

impl Default for TurnCursor {
    fn default() -> Self {
        Self {
            stdout_buf: Vec::new(),
            stderr_buf: Vec::new(),
            stdout_scratch: vec![0_u8; HARNESS_STREAM_CHUNK_BYTES],
            stderr_scratch: vec![0_u8; HARNESS_STREAM_CHUNK_BYTES],
            stdout_done: false,
            stderr_done: false,
        }
    }
}

pub(super) enum TurnProgress {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
}

fn take_ready_line(cursor: &mut TurnCursor) -> Option<TurnProgress> {
    if let Some(line) = take_line(&mut cursor.stdout_buf) {
        return Some(TurnProgress::Stdout(line));
    }
    if cursor.stdout_done && !cursor.stdout_buf.is_empty() {
        return Some(TurnProgress::Stdout(mem::take(&mut cursor.stdout_buf)));
    }
    if let Some(line) = take_line(&mut cursor.stderr_buf) {
        return Some(TurnProgress::Stderr(line));
    }
    if cursor.stderr_done && !cursor.stderr_buf.is_empty() {
        return Some(TurnProgress::Stderr(mem::take(&mut cursor.stderr_buf)));
    }
    None
}

// yields one line at a time from either stream, concurrently, until both reach EOF
pub(super) async fn next_turn_progress<Proc>(
    proc: &mut Proc,
    cursor: &mut TurnCursor,
) -> Result<Option<TurnProgress>, HarnessError>
where
    Proc: Process,
{
    loop {
        if let Some(progress) = take_ready_line(cursor) {
            return Ok(Some(progress));
        }
        if cursor.stdout_done && cursor.stderr_done {
            return Ok(None);
        }
        let done = PipeDone {
            stdout: cursor.stdout_done,
            stderr: cursor.stderr_done,
        };
        let (stdout, stderr) = read_step(
            proc,
            &mut cursor.stdout_scratch,
            &mut cursor.stderr_scratch,
            &done,
        )
        .await?;
        apply_chunk(stdout, &mut cursor.stdout_buf, &mut cursor.stdout_done);
        apply_chunk(stderr, &mut cursor.stderr_buf, &mut cursor.stderr_done);
    }
}
