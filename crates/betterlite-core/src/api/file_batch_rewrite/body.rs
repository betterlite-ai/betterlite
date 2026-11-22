use std::io::{self, BufRead, Seek, SeekFrom};
use std::sync::{Arc, Mutex, MutexGuard};

use thiserror::Error;
use tokio::task::JoinError;
use tokio_util::task::TaskTracker;

use crate::api::file_batch_guardrails::error::BatchGuardrailError;
use crate::api::file_batch_guardrails::lines::iter_lines;
use crate::api::file_batch_guardrails::record_json::load_record;
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

// One non-blank line as the record it must hold, under its physical line number
pub(super) fn record_of(
    line_number: usize,
    bytes: Vec<u8>,
) -> Result<ParsedRecord, BatchGuardrailError> {
    let payload = load_record(&bytes).map_err(|source| BatchGuardrailError::Record {
        line_number,
        source,
    })?;
    Ok(ParsedRecord {
        line_number,
        payload,
    })
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

    // Up to limit items numbered from first, read on the blocking port, with the physical lines this request consumed
    pub(super) async fn window<Item, Parse>(
        &self,
        tasks: &TaskTracker,
        first: usize,
        limit: usize,
        parse: Parse,
    ) -> Result<(Vec<Item>, usize), BodyError>
    where
        Source: BufRead,
        Item: Send + 'static,
        Parse: Fn(usize, Vec<u8>) -> Result<Item, BatchGuardrailError> + Send + 'static,
    {
        let body = self.clone();
        tasks
            .spawn_blocking(move || body.read_window(first, limit, parse))
            .await
            .map_err(|source| BodyError::Task { source })?
            .map_err(BodyError::from)
    }

    fn read_window<Item, Parse>(
        &self,
        first: usize,
        limit: usize,
        parse: Parse,
    ) -> Result<(Vec<Item>, usize), BatchGuardrailError>
    where
        Source: BufRead,
        Parse: Fn(usize, Vec<u8>) -> Result<Item, BatchGuardrailError>,
    {
        let mut state = self.lock();
        let base = first.saturating_sub(1);
        let mut kept = Vec::new();
        let mut consumed = 0_usize;
        for line in iter_lines(&mut *state).take(limit) {
            let line = line?;
            consumed = line.line_number;
            kept.push(parse(line.line_number.saturating_add(base), line.bytes)?);
        }
        drop(state);
        Ok((kept, consumed))
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

    pub(super) fn lock(&self) -> MutexGuard<'_, Source> {
        match self.inner.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}
