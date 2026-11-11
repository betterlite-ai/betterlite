use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

#[cfg(unix)]
use signal_hook::consts::{SIGINT, SIGTERM};
#[cfg(unix)]
use signal_hook::iterator::{Handle, Signals};
#[cfg(unix)]
use std::thread::{Builder, JoinHandle};

use super::error::CommandsError;

pub const SIGNAL_THREAD_NAME: &str = "betterlite-autoroute-signals";

pub trait StopGuard {
    fn wait(&self);
}

pub trait StopSignals {
    fn watch(&self) -> Result<Box<dyn StopGuard>, CommandsError>;
}

pub struct StopSignal {
    requested: AtomicBool,
    gate: Mutex<()>,
    ready: Condvar,
}

impl Default for StopSignal {
    fn default() -> Self {
        Self {
            requested: AtomicBool::new(false),
            gate: Mutex::new(()),
            ready: Condvar::new(),
        }
    }
}

impl StopSignal {
    // The lock pairs with the predicate check in wait, so a request never slips between the two
    pub fn request(&self) {
        let Ok(_guard) = self.gate.lock() else {
            return;
        };
        self.requested.store(true, Ordering::SeqCst);
        self.ready.notify_all();
    }

    pub fn wait(&self) {
        let Ok(mut guard) = self.gate.lock() else {
            return;
        };
        while !self.requested.load(Ordering::SeqCst) {
            match self.ready.wait(guard) {
                Ok(next) => guard = next,
                Err(_poisoned) => return,
            }
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
        let stop = Arc::new(StopSignal::default());
        let thread_stop = Arc::clone(&stop);
        // The iterator thread only sets the flag; allocation and file work stay in the main thread
        let thread = Builder::new()
            .name(SIGNAL_THREAD_NAME.to_owned())
            .spawn(move || {
                for _signal in signals.forever() {
                    thread_stop.request();
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
