use std::io::{self, BufRead, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex, MutexGuard};

use thiserror::Error;
use tokio::task::JoinError;
use tokio_util::task::TaskTracker;

use crate::api::file_batch_guardrails::error::BatchGuardrailError;
use crate::api::file_batch_guardrails::lines::{RawLine, iter_lines, iter_records};
use crate::api::file_batch_guardrails::records::ParsedRecord;

// What a blocking read ends with besides its window: the source's own refusal, or a blocking task the runtime cancelled
#[derive(Debug, Error)]
pub enum BodyError {
    #[error(transparent)]
    Input(#[from] BatchGuardrailError),
    #[error("the batch input file's blocking read task failed")]
    Task {
        #[source]
        source: JoinError,
    },
}

// The seekable upload body, shared by its caller and every pass: clones share one position, and each read, rewind and line copy runs on the caller's blocking port
#[derive(Debug)]
pub struct UploadBody<Source> {
    inner: Arc<Mutex<Source>>,
}

impl<Source> Clone for UploadBody<Source> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<Source> UploadBody<Source>
where
    Source: Send + 'static,
{
    #[must_use]
    pub fn new(source: Source) -> Self {
        Self {
            inner: Arc::new(Mutex::new(source)),
        }
    }

    // Up to limit records numbered from first, with the physical lines this request consumed
    pub(super) async fn records(
        &self,
        tasks: &TaskTracker,
        first: usize,
        limit: usize,
    ) -> Result<(Vec<ParsedRecord>, usize), BodyError>
    where
        Source: BufRead,
    {
        let body = self.clone();
        tasks
            .spawn_blocking(move || body.read_records(first, limit))
            .await
            .map_err(|source| BodyError::Task { source })?
            .map_err(BodyError::from)
    }

    // Up to limit raw lines numbered from first, with the physical lines this request consumed
    pub(super) async fn lines(
        &self,
        tasks: &TaskTracker,
        first: usize,
        limit: usize,
    ) -> Result<(Vec<RawLine>, usize), BodyError>
    where
        Source: BufRead,
    {
        let body = self.clone();
        tasks
            .spawn_blocking(move || body.read_lines(first, limit))
            .await
            .map_err(|source| BodyError::Task { source })?
            .map_err(BodyError::from)
    }

    // Offset 0 on the blocking port, so a disk-backed body never seeks on a runtime worker
    pub(super) async fn rewind(&self, tasks: &TaskTracker) -> io::Result<()>
    where
        Source: Seek,
    {
        let body = self.clone();
        match tasks
            .spawn_blocking(move || body.lock().seek(SeekFrom::Start(0)).map(drop))
            .await
        {
            Ok(sought) => sought,
            Err(source) => Err(io::Error::other(source)),
        }
    }

    fn read_records(
        &self,
        first: usize,
        limit: usize,
    ) -> Result<(Vec<ParsedRecord>, usize), BatchGuardrailError>
    where
        Source: BufRead,
    {
        let mut state = self.lock();
        let mut records = Vec::new();
        let parsed = {
            let mut counted = Counted {
                inner: &mut state,
                physical: 0,
            };
            let base = first.saturating_sub(1);
            for record in iter_records(&mut counted).take(limit) {
                match record {
                    Ok(mut record) => {
                        record.line_number = record.line_number.saturating_add(base);
                        records.push(record);
                    }
                    Err(BatchGuardrailError::Record {
                        line_number,
                        source,
                    }) => {
                        return Err(BatchGuardrailError::Record {
                            line_number: line_number.saturating_add(base),
                            source,
                        });
                    }
                    Err(read @ BatchGuardrailError::Read { .. }) => return Err(read),
                }
            }
            counted.physical
        };
        drop(state);
        Ok((records, parsed))
    }

    fn read_lines(
        &self,
        first: usize,
        limit: usize,
    ) -> Result<(Vec<RawLine>, usize), BatchGuardrailError>
    where
        Source: BufRead,
    {
        let mut state = self.lock();
        let mut lines = Vec::new();
        let read = {
            let mut counted = Counted {
                inner: &mut state,
                physical: 0,
            };
            let base = first.saturating_sub(1);
            for line in iter_lines(&mut counted).take(limit) {
                let mut line = line?;
                line.line_number = line.line_number.saturating_add(base);
                lines.push(line);
            }
            counted.physical
        };
        drop(state);
        Ok((lines, read))
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, Source> {
        match self.inner.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

// Counts the physical lines one request consumed, blank lines included, so the next request keeps numbering from the file's start
struct Counted<'source, Source> {
    inner: &'source mut Source,
    physical: usize,
}

impl<Source> Read for Counted<'_, Source>
where
    Source: Read,
{
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl<Source> BufRead for Counted<'_, Source>
where
    Source: BufRead,
{
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amount: usize) {
        self.inner.consume(amount);
    }

    fn read_until(&mut self, byte: u8, buffer: &mut Vec<u8>) -> io::Result<usize> {
        let read = self.inner.read_until(byte, buffer)?;
        if read > 0 {
            self.physical = self.physical.saturating_add(1);
        }
        Ok(read)
    }
}
