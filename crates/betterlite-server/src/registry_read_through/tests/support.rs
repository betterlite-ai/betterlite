use std::future::{Future, ready};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use tracing::subscriber::{DefaultGuard, set_default};

use crate::registry_read_through::{Resync, ResyncError};

pub(crate) struct CountingResync {
    pub(crate) calls: Arc<AtomicUsize>,
    pub(crate) found: bool,
}

impl Resync for CountingResync {
    fn resync(&self, _key: &str) -> impl Future<Output = Result<bool, ResyncError>> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ready(Ok(self.found))
    }
}

pub(crate) struct FailingResync {
    pub(crate) calls: Arc<AtomicUsize>,
    pub(crate) message: &'static str,
}

impl Resync for FailingResync {
    fn resync(&self, _key: &str) -> impl Future<Output = Result<bool, ResyncError>> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ready(Err(ResyncError::Failed {
            message: self.message.to_owned(),
        }))
    }
}

pub(crate) fn record<T>(calls: &Mutex<Vec<T>>, call: T) {
    calls.lock().unwrap().push(call);
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
