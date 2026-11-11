use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::park_timeout;
use std::time::Duration;

#[cfg(unix)]
use signal_hook::consts::{SIGINT, SIGTERM};
#[cfg(unix)]
use signal_hook::iterator::{Handle, Signals};
#[cfg(unix)]
use std::thread::{Builder, JoinHandle};

use super::error::CommandsError;

pub trait StopGuard {
    fn wait(&self);
}

pub trait StopSignals {
    fn watch(&self) -> Result<Box<dyn StopGuard>, CommandsError>;
}

pub struct StopSignal {
    pub(crate) requested: AtomicBool,
}

impl StopSignal {
    // The flag is the only thing a signal path touches; the foreground wait polls it briefly
    pub fn wait(&self) {
        while !self.requested.load(Ordering::SeqCst) {
            park_timeout(Duration::from_millis(50));
        }
    }
}

#[cfg(unix)]
pub struct OsSignals;

#[cfg(unix)]
impl StopSignals for OsSignals {
    fn watch(&self) -> Result<Box<dyn StopGuard>, CommandsError> {
        let mut signals =
            Signals::new([SIGINT, SIGTERM]).map_err(|source| CommandsError::Signals { source })?;
        let handle = signals.handle();
        let stop = Arc::new(StopSignal {
            requested: AtomicBool::new(false),
        });
        let thread_stop = Arc::clone(&stop);
        // The iterator thread only sets the flag; allocation and file work stay in the main thread
        let thread = Builder::new()
            .spawn(move || {
                for _signal in signals.forever() {
                    thread_stop.requested.store(true, Ordering::SeqCst);
                }
            })
            .map_err(|source| CommandsError::Signals { source })?;
        Ok(Box::new(OsStopGuard {
            stop,
            handle,
            thread: Some(thread),
        }))
    }
}

#[cfg(unix)]
struct OsStopGuard {
    stop: Arc<StopSignal>,
    handle: Handle,
    thread: Option<JoinHandle<()>>,
}

#[cfg(unix)]
impl StopGuard for OsStopGuard {
    fn wait(&self) {
        self.stop.wait();
    }
}

#[cfg(unix)]
impl Drop for OsStopGuard {
    fn drop(&mut self) {
        self.handle.close();
        if let Some(thread) = self.thread.take() {
            let _joined = thread.join();
        }
    }
}

#[cfg(not(unix))]
pub struct OsSignals;

#[cfg(not(unix))]
impl StopSignals for OsSignals {
    fn watch(&self) -> Result<Box<dyn StopGuard>, CommandsError> {
        Err(CommandsError::SignalsUnsupported)
    }
}
