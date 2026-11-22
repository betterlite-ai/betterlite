use std::io::{self, Read, Seek, SeekFrom, Write};
use std::num::TryFromIntError;
use std::sync::{Arc, Mutex, MutexGuard};

use tempfile::SpooledTempFile;
use thiserror::Error;
use tokio::task::JoinError;
use tokio_util::task::TaskTracker;

use crate::api::file_batch_guardrails::REWRITE_SPOOL_BYTES;

// One owned temporary file: clones share it, all its disk work runs on the caller's blocking port, and closing any clone closes it for every clone
#[derive(Debug, Clone)]
pub struct Spool {
    pub(super) inner: Arc<Mutex<Option<SpooledTempFile>>>,
}

#[derive(Debug, Error)]
pub enum SpoolError {
    #[error("writing to the spool failed")]
    Write {
        #[source]
        source: io::Error,
    },
    #[error("reading from the spool failed")]
    Read {
        #[source]
        source: io::Error,
    },
    #[error("the spool was already closed")]
    Closed,
    #[error("a spooled record is larger than this platform addresses")]
    Length {
        #[source]
        source: TryFromIntError,
    },
    #[error("the spool's blocking I/O task failed")]
    Task {
        #[source]
        source: JoinError,
    },
}

impl Default for Spool {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Some(SpooledTempFile::new(REWRITE_SPOOL_BYTES)))),
        }
    }
}

impl Spool {
    // The offset the bytes were appended at, which is what a redaction record carries instead of its text
    pub async fn append(&self, tasks: &TaskTracker, bytes: Vec<u8>) -> Result<u64, SpoolError> {
        let spool = self.clone();
        tasks
            .spawn_blocking(move || spool.write_at_end(&bytes))
            .await
            .map_err(|source| SpoolError::Task { source })?
    }

    pub async fn read_at(
        &self,
        tasks: &TaskTracker,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, SpoolError> {
        let spool = self.clone();
        tasks
            .spawn_blocking(move || spool.read_span(offset, length))
            .await
            .map_err(|source| SpoolError::Task { source })?
    }

    // Reads from where the file stands now, which is 0 after a rewrite hands its output back
    pub async fn read_to_end(&self, tasks: &TaskTracker) -> Result<Vec<u8>, SpoolError> {
        let spool = self.clone();
        tasks
            .spawn_blocking(move || spool.read_rest())
            .await
            .map_err(|source| SpoolError::Task { source })?
    }

    pub async fn rewind(&self, tasks: &TaskTracker) -> Result<(), SpoolError> {
        let spool = self.clone();
        tasks
            .spawn_blocking(move || spool.seek_start())
            .await
            .map_err(|source| SpoolError::Task { source })?
    }

    pub fn close(&self) {
        let mut state = self.lock();
        let file = state.take();
        drop(state);
        drop(file);
    }

    #[must_use]
    pub fn is_closed(&self) -> bool {
        match self.inner.lock() {
            Ok(state) => state.is_none(),
            Err(poisoned) => poisoned.into_inner().is_none(),
        }
    }

    #[must_use]
    pub fn is_rolled(&self) -> bool {
        match self.inner.lock() {
            Ok(state) => state.as_ref().is_some_and(SpooledTempFile::is_rolled),
            Err(poisoned) => poisoned
                .into_inner()
                .as_ref()
                .is_some_and(SpooledTempFile::is_rolled),
        }
    }

    fn write_at_end(&self, bytes: &[u8]) -> Result<u64, SpoolError> {
        let mut state = self.lock();
        let file = state.as_mut().ok_or(SpoolError::Closed)?;
        let offset = file
            .seek(SeekFrom::End(0))
            .map_err(|source| SpoolError::Write { source })?;
        file.write_all(bytes)
            .map_err(|source| SpoolError::Write { source })?;
        Ok(offset)
    }

    fn read_span(&self, offset: u64, length: u64) -> Result<Vec<u8>, SpoolError> {
        let mut state = self.lock();
        let file = state.as_mut().ok_or(SpoolError::Closed)?;
        let size = usize::try_from(length).map_err(|source| SpoolError::Length { source })?;
        let mut bytes = vec![0_u8; size];
        file.seek(SeekFrom::Start(offset))
            .map_err(|source| SpoolError::Read { source })?;
        file.read_exact(&mut bytes)
            .map_err(|source| SpoolError::Read { source })?;
        Ok(bytes)
    }

    fn read_rest(&self) -> Result<Vec<u8>, SpoolError> {
        let mut state = self.lock();
        let file = state.as_mut().ok_or(SpoolError::Closed)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|source| SpoolError::Read { source })?;
        Ok(bytes)
    }

    fn seek_start(&self) -> Result<(), SpoolError> {
        let mut state = self.lock();
        let file = state.as_mut().ok_or(SpoolError::Closed)?;
        file.seek(SeekFrom::Start(0))
            .map(drop)
            .map_err(|source| SpoolError::Write { source })
    }

    fn lock(&self) -> MutexGuard<'_, Option<SpooledTempFile>> {
        match self.inner.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

// Keeps LiteLLM's close on every scan and rewrite path, cancellation included, unless the caller keeps the spool
pub(super) struct OwnedSpool(pub(super) Option<Spool>);

impl Drop for OwnedSpool {
    fn drop(&mut self) {
        if let Some(spool) = self.0.take() {
            spool.close();
        }
    }
}
