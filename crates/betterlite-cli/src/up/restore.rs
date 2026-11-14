use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_logging::private_json::files::PrivateFiles;

use crate::settings_backup::report::restore_and_report;

// LiteLLM reaches this cleanup from both the signal path and atexit, so the latch keeps the second call silent
pub(super) struct RestoreOnce<'guard> {
    pub(super) home: &'guard Path,
    pub(super) files: &'guard PrivateFiles,
    pub(super) stdout: &'guard mut dyn Write,
    pub(super) stderr: &'guard mut dyn Write,
    pub(super) done: AtomicBool,
}

impl RestoreOnce<'_> {
    pub(super) fn restore(&mut self) {
        if self.done.swap(true, Ordering::SeqCst) {
            return;
        }
        let Err(error) = restore_and_report(self.home, self.files, &mut *self.stdout) else {
            return;
        };
        let _unused = writeln!(self.stderr, "{error}");
    }
}

impl Drop for RestoreOnce<'_> {
    fn drop(&mut self) {
        self.restore();
    }
}
