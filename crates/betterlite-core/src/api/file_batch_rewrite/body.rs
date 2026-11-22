use std::io::{self, BufRead, Seek, SeekFrom};
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

    // The request stops right after its last record, so that record's line number is how many lines the file has moved on
    fn read_records(
        &self,
        first: usize,
        limit: usize,
    ) -> Result<(Vec<ParsedRecord>, usize), BatchGuardrailError>
    where
        Source: BufRead,
    {
        let mut state = self.lock();
        let base = first.saturating_sub(1);
        let mut records = Vec::new();
        for record in iter_records(&mut *state).take(limit) {
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
        let consumed = records
            .last()
            .map_or(0, |record| record.line_number.saturating_sub(base));
        drop(state);
        Ok((records, consumed))
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
        let base = first.saturating_sub(1);
        let mut lines = Vec::new();
        for line in iter_lines(&mut *state).take(limit) {
            let mut line = line?;
            line.line_number = line.line_number.saturating_add(base);
            lines.push(line);
        }
        let consumed = lines
            .last()
            .map_or(0, |line| line.line_number.saturating_sub(base));
        drop(state);
        Ok((lines, consumed))
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, Source> {
        match self.inner.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}
