mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, DirEntry, File};
use std::io::{self, Read};
use std::path::{MAIN_SEPARATOR, Path, PathBuf};
use std::str;

use sha2::{Digest as _, Sha256};
use similar::TextDiff;
use thiserror::Error;
use tokio::task::{JoinError, JoinSet};

use crate::events::FileChange;
use crate::sandbox::Sandbox;
use crate::types::FileChangeKind;

pub const MAX_FILE_BYTES: usize = 50 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_DIFF_BYTES: usize = 256 * 1024;
pub const SKIP_DIRS: [&str; 8] = [
    ".git",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
];
const HASH_CHUNK_BYTES: usize = 1024 * 1024;

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("the workspace snapshot task failed")]
    Walk {
        #[source]
        source: JoinError,
    },
    #[error("the workspace snapshot task ended without a result")]
    NoResult,
}

#[must_use]
pub fn snapshot_local_sync(root: &Path) -> BTreeMap<String, String> {
    let mut snapshot = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        // os.walk skips a folder it cannot list, and so does the snapshot
        let Ok(listing) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in listing.flatten() {
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                snapshot.extend(hash_entry(root, &entry));
                continue;
            }
            let name = entry.file_name();
            if !SKIP_DIRS.iter().any(|skipped| name == *skipped) {
                pending.push(entry.path());
            }
        }
    }
    snapshot
}

fn hash_entry(root: &Path, entry: &DirEntry) -> Option<(String, String)> {
    // DirEntry::metadata does not follow links, so a symlink is never a regular file here
    let metadata = entry.metadata().ok()?;
    let size = usize::try_from(metadata.len()).ok()?;
    if !metadata.is_file() || size > MAX_FILE_BYTES {
        return None;
    }
    let path = entry.path();
    let mut file = File::open(&path).ok()?;
    let digest = hash_reader(&mut file)?;
    let relative = path.strip_prefix(root).ok()?.to_str()?;
    Some((relative.replace(MAIN_SEPARATOR, "/"), hex::encode(digest)))
}

fn hash_reader<Reader>(reader: &mut Reader) -> Option<[u8; 32]>
where
    Reader: Read,
{
    let mut digest = Sha256::new();
    let mut chunk = vec![0_u8; HASH_CHUNK_BYTES];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => digest.update(chunk.get(..count)?),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(<[u8; 32]>::from(digest.finalize()))
}

pub async fn snapshot_local(root: PathBuf) -> Result<BTreeMap<String, String>, SnapshotError> {
    let mut walks = JoinSet::new();
    walks.spawn_blocking(move || snapshot_local_sync(&root));
    match walks.join_next().await {
        Some(Ok(snapshot)) => Ok(snapshot),
        Some(Err(source)) => Err(SnapshotError::Walk { source }),
        None => Err(SnapshotError::NoResult),
    }
}

#[must_use]
pub fn diff_snapshots(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Vec<(String, FileChangeKind)> {
    let paths = before.keys().chain(after.keys()).collect::<BTreeSet<_>>();
    let mut changes = Vec::new();
    for path in paths {
        let kind = match (before.get(path), after.get(path)) {
            (None, _) => FileChangeKind::Created,
            (Some(_), None) => FileChangeKind::Deleted,
            (Some(old), Some(new)) if old != new => FileChangeKind::Modified,
            (Some(_), Some(_)) => continue,
        };
        changes.push((path.clone(), kind));
    }
    changes
}

fn diffable_text(data: &[u8]) -> Option<&str> {
    if data.len() > MAX_DIFF_BYTES || data.contains(&0) {
        return None;
    }
    str::from_utf8(data).ok()
}

#[must_use]
pub fn unified_diff(path: &str, old: Option<&str>, new: Option<&str>) -> String {
    let from = old.map_or_else(|| "/dev/null".to_owned(), |_| format!("a/{path}"));
    let to = new.map_or_else(|| "/dev/null".to_owned(), |_| format!("b/{path}"));
    TextDiff::from_lines(old.unwrap_or_default(), new.unwrap_or_default())
        .unified_diff()
        .header(&from, &to)
        .to_string()
}

pub async fn capture_text_contents<Workspace>(
    sandbox: &Workspace,
    paths_hashes: &BTreeMap<String, String>,
) -> BTreeMap<String, Vec<u8>>
where
    Workspace: Sandbox,
{
    let mut captured = BTreeMap::new();
    let mut total = 0_usize;
    for path in paths_hashes.keys() {
        if total >= MAX_TOTAL_BYTES {
            break;
        }
        // LiteLLM treats a file it cannot read as one it never saw
        let Ok(data) = sandbox.read(path, MAX_FILE_BYTES).await else {
            continue;
        };
        total = total.saturating_add(data.len());
        if diffable_text(&data).is_some() {
            captured.insert(path.clone(), data);
        }
    }
    captured
}

async fn change_diff<Workspace>(
    sandbox: &Workspace,
    path: &str,
    kind: FileChangeKind,
    old: Option<&str>,
) -> Option<String>
where
    Workspace: Sandbox,
{
    if kind == FileChangeKind::Deleted {
        return old.map(|text| unified_diff(path, Some(text), None));
    }
    let current = sandbox.read(path, MAX_DIFF_BYTES).await.ok()?;
    let new = diffable_text(&current)?;
    match kind {
        FileChangeKind::Modified => old.map(|text| unified_diff(path, Some(text), Some(new))),
        FileChangeKind::Created | FileChangeKind::Deleted => {
            Some(unified_diff(path, None, Some(new)))
        }
    }
}

pub async fn build_file_changes<Workspace>(
    sandbox: &Workspace,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
    before_contents: &BTreeMap<String, Vec<u8>>,
) -> Vec<FileChange>
where
    Workspace: Sandbox,
{
    let mut changes = Vec::new();
    for (path, kind) in diff_snapshots(before, after) {
        let old = before_contents
            .get(&path)
            .and_then(|data| diffable_text(data));
        let diff = change_diff(sandbox, &path, kind, old).await;
        changes.push(FileChange { path, kind, diff });
    }
    changes
}
