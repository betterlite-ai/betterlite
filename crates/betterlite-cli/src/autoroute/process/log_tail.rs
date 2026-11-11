use std::fs::File;
use std::io::{BufRead as _, BufReader, Write, stdout};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{Builder, JoinHandle};
use std::time::Duration;

use super::control::{Clock, RealClock};
use super::error::ProcessError;

pub const LOG_WAIT_INTERVAL: Duration = Duration::from_millis(100);
pub const LOG_POLL_INTERVAL: Duration = Duration::from_millis(200);
pub const LOG_THREAD_NAME: &str = "betterlite-autoroute-log";

pub struct LogTailer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

pub fn spawn_log_tail(log_path: &Path) -> Result<LogTailer, ProcessError> {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let path = log_path.to_owned();
    let thread = Builder::new()
        .name(LOG_THREAD_NAME.to_owned())
        .spawn(move || {
            let mut out = stdout();
            stream_log(&path, &thread_stop, &RealClock, &mut out);
        })
        .map_err(|source| ProcessError::LogThread { source })?;
    Ok(LogTailer {
        stop,
        thread: Some(thread),
    })
}

impl Drop for LogTailer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _joined = thread.join();
        }
    }
}

// The log appears once the gateway opens it, so the tail waits for the file and prints lines as they arrive
pub fn stream_log(log_path: &Path, stop: &AtomicBool, clock: &dyn Clock, out: &mut dyn Write) {
    while !log_path.exists() && !stop.load(Ordering::SeqCst) {
        clock.sleep(LOG_WAIT_INTERVAL);
    }
    if stop.load(Ordering::SeqCst) || !log_path.exists() {
        return;
    }
    let Ok(file) = File::open(log_path) else {
        return;
    };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    while !stop.load(Ordering::SeqCst) {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => clock.sleep(LOG_POLL_INTERVAL),
            Ok(_read) => {
                let _written = out.write_all(line.as_bytes());
                let _flushed = out.flush();
            }
            Err(_read_error) => return,
        }
    }
}
