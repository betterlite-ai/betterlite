use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

const TEMP_ROOT_ENV: [&str; 3] = ["TMPDIR", "TEMP", "TMP"];
static TEMP_ROOT_PROBE_ID: AtomicU64 = AtomicU64::new(0);

pub(super) fn resolve(environment: &BTreeMap<String, String>) -> PathBuf {
    TEMP_ROOT_ENV
        .into_iter()
        .filter_map(|key| environment.get(key))
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .chain(default_roots())
        .find_map(|candidate| usable_root(&candidate))
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub(super) fn multiproc_directory(
    environment: &BTreeMap<String, String>,
    configured: Option<&String>,
) -> PathBuf {
    configured.map_or_else(
        || {
            let root = resolve(environment);
            multiproc_path(&root)
        },
        PathBuf::from,
    )
}

fn multiproc_path(root: &Path) -> PathBuf {
    if root.as_os_str() == "." {
        PathBuf::from("litellm_prometheus_multiproc")
    } else {
        root.join("litellm_prometheus_multiproc")
    }
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
    candidates.push(env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    candidates
}

fn usable_root(candidate: &Path) -> Option<PathBuf> {
    let candidate = if candidate.as_os_str() == "." {
        candidate.to_path_buf()
    } else {
        absolute_lexical_path(candidate)?
    };
    if !candidate.is_dir() {
        return None;
    }
    let probe_id = TEMP_ROOT_PROBE_ID.fetch_add(1, Ordering::Relaxed);
    let probe = candidate.join(format!(
        ".betterlite-temp-probe-{}-{probe_id}",
        process::id()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .ok()?;
    let result = file.write_all(b"tempdir probe");
    drop(file);
    let removed = fs::remove_file(probe);
    result.ok()?;
    removed.ok()?;
    Some(candidate)
}

fn absolute_lexical_path(candidate: &Path) -> Option<PathBuf> {
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        env::current_dir().ok()?.join(candidate)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(name) => normalized.push(name),
        }
    }
    Some(normalized)
}
