use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_logging::private_json::files::PrivateFiles;

use crate::autoroute::process::control::{Clock, SignalPort, terminate};
use crate::autoroute::process::log_tail::TailWorker;
use crate::autoroute::process::pid_record::clear_pid_record;
use crate::settings_backup::restore::restore_claude_settings;

pub struct TeardownRequest<'request> {
    pub settings: &'request Path,
    pub backup: &'request Path,
    pub pid_record: &'request Path,
    pub pid: u32,
    pub port: u16,
    pub signal_port: &'request dyn SignalPort,
    pub clock: &'request dyn Clock,
    pub files: &'request PrivateFiles,
    pub tail: Option<Box<dyn TailWorker>>,
    pub stdout: &'request mut dyn Write,
    pub stderr: &'request mut dyn Write,
}

// The only path that terminates the proxy, clears the PID record and restores the settings
#[must_use]
pub fn run_teardown(mut request: TeardownRequest<'_>, done: &AtomicBool) -> bool {
    // LiteLLM's Lock.acquire(blocking=False): only the first caller cleans up and the rest return
    if done.swap(true, Ordering::SeqCst) {
        return false;
    }
    terminate(request.pid, request.signal_port, request.clock);
    let _cleared = clear_pid_record(request.pid_record);
    match restore_claude_settings(request.settings, request.backup, request.files) {
        Ok(_restored) => {
            let _done = writeln!(
                request.stdout,
                "\nStopped ephemeral proxy and restored Claude Code settings.\nRestart any Claude Code session still open from this session, or another local account could bind the now-free port {} and receive its requests. Do not use `lite autoroute start` on a shared or multi-tenant host.",
                request.port
            );
        }
        Err(error) => {
            let _reported = writeln!(request.stderr, "{error}");
        }
    }
    drop(request.tail.take());
    true
}
