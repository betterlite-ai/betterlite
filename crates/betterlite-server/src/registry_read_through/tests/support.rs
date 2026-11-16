use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tracing::subscriber::{DefaultGuard, set_default};

use crate::registry_read_through::{Resync, ResyncError};

pub(crate) struct CountingResync {
    pub(crate) calls: Arc<AtomicUsize>,
    pub(crate) found: bool,
}

impl Resync for CountingResync {
    async fn resync(&self, _key: &str) -> Result<bool, ResyncError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.found)
    }
}

pub(crate) struct FailingResync {
    pub(crate) calls: Arc<AtomicUsize>,
    pub(crate) message: &'static str,
}

impl Resync for FailingResync {
    async fn resync(&self, _key: &str) -> Result<bool, ResyncError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(ResyncError::Failed {
            message: self.message.to_owned(),
        })
    }
}

pub(crate) fn capture_logs() -> (tempfile::NamedTempFile, DefaultGuard) {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    (log, set_default(subscriber))
}
