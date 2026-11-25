use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use betterlite_prometheus::multiprocess::MultiprocessError;
use betterlite_prometheus::multiprocess::collector::mark_process_dead;
use nix::errno::Errno;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

mod tests;

const LIVE_GAUGE_PREFIX: &str = "gauge_live";

#[derive(Debug, thiserror::Error)]
pub enum CleanupError {
    #[error("cannot list the Prometheus multiprocess folder {}", path.display())]
    List {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot mark Prometheus worker {pid} as dead")]
    MarkDead {
        pid: u32,
        #[source]
        source: MultiprocessError,
    },
}

// glob's *.db: every entry whose name ends in .db except dot files, and one that can't be removed is only a warning
pub fn wipe_directory(directory: &Path) -> Result<usize, CleanupError> {
    let mut deleted = 0_usize;
    for name in entry_names(directory)? {
        let bytes = name.as_encoded_bytes();
        if bytes.starts_with(b".") || !bytes.ends_with(b".db") {
            continue;
        }
        let path = directory.join(name);
        match fs::remove_file(&path) {
            Ok(()) => deleted = deleted.saturating_add(1),
            Err(error) => tracing::warn!(
                "Failed to delete stale prometheus file {}: {error}",
                path.display()
            ),
        }
    }
    if deleted > 0 {
        tracing::info!(
            "Prometheus cleanup: wiped {deleted} stale .db files from {}",
            directory.display()
        );
    }
    Ok(deleted)
}

// the gunicorn child_exit hook: nothing to clean when the proxy runs without PROMETHEUS_MULTIPROC_DIR
pub fn mark_worker_exit(worker_pid: u32, multiproc_dir: Option<&Path>) {
    let Some(directory) = multiproc_dir.filter(|folder| !folder.as_os_str().is_empty()) else {
        return;
    };
    match mark_process_dead(worker_pid, directory) {
        Ok(()) => tracing::info!("Prometheus cleanup: marked worker {worker_pid} as dead"),
        Err(error) => {
            tracing::warn!("Failed to mark prometheus worker {worker_pid} as dead: {error}");
        }
    }
}

// os.kill(pid, 0): only ESRCH means gone, EPERM is a process another user owns, and no process holds a pid past pid_t
#[must_use]
pub fn is_running(pid: u32) -> bool {
    let Ok(raw) = i32::try_from(pid) else {
        return false;
    };
    !matches!(kill(Pid::from_raw(raw), None::<Signal>), Err(Errno::ESRCH))
}

// a replacement worker's startup: uvicorn has no exit hook, so the live gauges of crashed workers go here
pub fn mark_dead_workers(directory: &Path, own_pid: u32) -> Result<Vec<u32>, CleanupError> {
    let owners: BTreeSet<u32> = entry_names(directory)?
        .iter()
        .filter_map(|name| name.to_str().and_then(live_gauge_pid))
        .collect();
    let dead: Vec<u32> = owners
        .into_iter()
        .filter(|pid| *pid != own_pid && !is_running(*pid))
        .collect();
    if dead.is_empty() {
        return Ok(dead);
    }
    for pid in &dead {
        mark_process_dead(*pid, directory)
            .map_err(|source| CleanupError::MarkDead { pid: *pid, source })?;
    }
    tracing::info!(
        "Prometheus cleanup: marked dead workers {dead:?} in {}",
        directory.display()
    );
    Ok(dead)
}

// glob's gauge_live*_*.db, then _LIVE_GAUGE_PID's search for gauge_live, a lowercase mode, _, the pid and .db at the end
fn live_gauge_pid(name: &str) -> Option<u32> {
    if !name.starts_with(LIVE_GAUGE_PREFIX) {
        return None;
    }
    let (head, digits) = name.strip_suffix(".db")?.rsplit_once('_')?;
    let mode_start = head
        .rfind(LIVE_GAUGE_PREFIX)?
        .checked_add(LIVE_GAUGE_PREFIX.len())?;
    let mode = head.get(mode_start..)?;
    let well_formed = digits.bytes().all(|byte| byte.is_ascii_digit())
        && mode.bytes().all(|byte| byte.is_ascii_lowercase());
    if !well_formed {
        return None;
    }
    digits.parse().ok()
}

fn entry_names(directory: &Path) -> Result<Vec<OsString>, CleanupError> {
    let listing_error = |source: io::Error| CleanupError::List {
        path: directory.to_path_buf(),
        source,
    };
    let entries = fs::read_dir(directory).map_err(listing_error)?;
    entries
        .map(|entry| entry.map(|found| found.file_name()).map_err(listing_error))
        .collect()
}
