use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, ThreadId};

// Aims an interrupt only while the worker is inside the sandboxed call, so a recycled thread id is never hit.
#[derive(Debug, Clone)]
pub struct InterruptGate {
    pub thread: Arc<Mutex<Option<ThreadId>>>,
    pub interrupted: Arc<AtomicBool>,
}

impl InterruptGate {
    #[must_use]
    pub fn new() -> Self {
        Self {
            thread: Arc::new(Mutex::new(None)),
            interrupted: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn open(&self) {
        self.interrupted.store(false, Ordering::SeqCst);
        *self.thread.lock().unwrap_or_else(PoisonError::into_inner) = Some(thread::current().id());
    }

    pub fn close(&self) {
        self.interrupted.store(false, Ordering::SeqCst);
        *self.thread.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    #[must_use]
    pub fn interrupt(&self) -> bool {
        if self
            .thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none()
        {
            return false;
        }
        self.interrupted.store(true, Ordering::SeqCst);
        true
    }
}

impl Default for InterruptGate {
    fn default() -> Self {
        Self::new()
    }
}
