use std::fs;
use std::path::Path;
use std::time::Duration;

use super::control::Clock;
use super::error::ProcessError;
use super::launcher::LaunchedProcess;

pub const LIVELINESS_PATH: &str = "/health/liveliness";
pub const LIVELINESS_TIMEOUT: Duration = Duration::from_secs(30);
pub const LIVELINESS_POLL_INTERVAL: Duration = Duration::from_millis(500);
pub const LOG_TAIL_LINES: usize = 40;
pub const NO_LOG_OUTPUT: &str = "(no log output captured)";

pub trait LivelinessProbe {
    fn is_live(&self, url: &str) -> bool;
}

// LiteLLM polls until the endpoint answers 200, the proxy exits or 30 seconds pass, then tails the log
pub fn poll_liveliness(
    base_url: &str,
    log_path: &Path,
    process: &mut dyn LaunchedProcess,
    probe: &dyn LivelinessProbe,
    clock: &dyn Clock,
) -> Result<(), ProcessError> {
    let url = format!("{}{LIVELINESS_PATH}", base_url.trim_end_matches('/'));
    let deadline = clock.now().checked_add(LIVELINESS_TIMEOUT);
    while deadline.is_some_and(|end| clock.now() < end) {
        if let Some(exit) = process.has_exited()? {
            let code = exit
                .code
                .map_or_else(|| "unknown".to_owned(), |number| number.to_string());
            return Err(ProcessError::ProxyExitedEarly {
                code,
                tail: tail_log(log_path),
            });
        }
        if probe.is_live(&url) {
            return Ok(());
        }
        clock.sleep(LIVELINESS_POLL_INTERVAL);
    }
    Err(ProcessError::ProxyNeverHealthy {
        timeout: LIVELINESS_TIMEOUT.as_secs(),
        tail: tail_log(log_path),
    })
}

#[must_use]
pub fn tail_log(log_path: &Path) -> String {
    if !log_path.exists() {
        return NO_LOG_OUTPUT.to_owned();
    }
    let Ok(bytes) = fs::read(log_path) else {
        return NO_LOG_OUTPUT.to_owned();
    };
    #[expect(
        clippy::disallowed_methods,
        reason = "LiteLLM reads the log with errors=\"replace\" and still shows its last lines"
    )]
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let tail: Vec<&str> = lines
        .iter()
        .rev()
        .take(LOG_TAIL_LINES)
        .rev()
        .copied()
        .collect();
    tail.join("\n")
}
