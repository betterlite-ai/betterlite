use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const TEMP_ROOT_ENV: [&str; 3] = ["TMPDIR", "TEMP", "TMP"];
static TEMP_ROOT_PROBE_ID: AtomicU64 = AtomicU64::new(0);

pub(super) fn resolve(environment: &BTreeMap<String, String>) -> PathBuf {
    let mut candidates = TEMP_ROOT_ENV
        .into_iter()
        .filter_map(|key| environment.get(key))
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    candidates.extend(default_roots());
    candidates
        .into_iter()
        .find_map(|candidate| usable_root(&candidate))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn default_roots() -> Vec<PathBuf> {
    #[cfg(unix)]
    let mut candidates = ["/tmp", "/var/tmp", "/usr/tmp"]
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    #[cfg(windows)]
    let mut candidates = [r"C:\TEMP", r"C:\TMP", r"\TEMP", r"\TMP"]
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    #[cfg(not(any(unix, windows)))]
    let mut candidates = Vec::new();
    candidates.push(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    candidates
}

fn usable_root(candidate: &Path) -> Option<PathBuf> {
    if !candidate.is_dir() {
        return None;
    }
    let probe_id = TEMP_ROOT_PROBE_ID.fetch_add(1, Ordering::Relaxed);
    let probe = candidate.join(format!(
        ".betterlite-temp-probe-{}-{probe_id}",
        std::process::id()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .ok()?;
    let result = file.write_all(b"tempdir probe");
    drop(file);
    drop(fs::remove_file(probe));
    result.ok()?;
    if candidate == Path::new(".") {
        Some(candidate.to_path_buf())
    } else {
        candidate.canonicalize().ok()
    }
}
