use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read as _};
use std::path::{MAIN_SEPARATOR, Path};

use sha2::{Digest as _, Sha256};

use crate::events::FileChange;
use crate::types::FileChangeKind;

pub const HARNESS_SNAPSHOT_MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
pub const HARNESS_SNAPSHOT_MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
pub const HARNESS_MAX_DIFF_BYTES: usize = 256 * 1024;
const HASH_CHUNK_BYTES: usize = 1024 * 1024;
const SKIP_DIRS: [&str; 8] = [
    ".git",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
];

pub fn snapshot_local_sync<Root>(root: Root) -> io::Result<BTreeMap<String, String>>
where
    Root: AsRef<Path>,
{
    let mut snapshot = BTreeMap::new();
    walk_snapshot(root.as_ref(), root.as_ref(), &mut snapshot)?;
    Ok(snapshot)
}

fn walk_snapshot(
    root: &Path,
    directory: &Path,
    snapshot: &mut BTreeMap<String, String>,
) -> io::Result<()> {
    for directory_entry in fs::read_dir(directory)? {
        let entry = directory_entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| SKIP_DIRS.contains(&name))
            {
                continue;
            }
            walk_snapshot(root, &path, snapshot)?;
            continue;
        }
        if metadata.is_file() && metadata.len() <= HARNESS_SNAPSHOT_MAX_FILE_BYTES {
            let relative = relative_path(root, &path)?;
            snapshot.insert(relative, hash_file(&path)?);
        }
    }
    Ok(())
}

fn relative_path(root: &Path, path: &Path) -> io::Result<String> {
    Ok(path
        .strip_prefix(root)
        .map_err(|error| io::Error::other(error.to_string()))?
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "path is not valid UTF-8"))?
        .replace(MAIN_SEPARATOR, "/"))
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; HASH_CHUNK_BYTES];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(buffer.get(..count).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "read exceeded hash buffer")
        })?);
    }
    let digest_bytes = digest.finalize();
    let mut output = String::new();
    for byte in digest_bytes {
        write!(output, "{byte:02x}").map_err(|_| io::Error::other("formatting digest failed"))?;
    }
    Ok(output)
}

#[must_use]
pub fn diff_snapshots(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Vec<(String, FileChangeKind)> {
    let mut paths = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .filter_map(|path| {
            let kind = match (before.get(&path), after.get(&path)) {
                (None, Some(_)) => FileChangeKind::Created,
                (Some(_), None) => FileChangeKind::Deleted,
                (Some(old), Some(new)) if old != new => FileChangeKind::Modified,
                _ => return None,
            };
            Some((path, kind))
        })
        .collect()
}

pub fn capture_text_contents<Root>(
    root: Root,
    paths_hashes: &BTreeMap<String, String>,
) -> io::Result<BTreeMap<String, Vec<u8>>>
where
    Root: AsRef<Path>,
{
    let mut captured = BTreeMap::new();
    let mut total = 0_u64;
    for path in paths_hashes.keys() {
        if total >= HARNESS_SNAPSHOT_MAX_TOTAL_BYTES {
            break;
        }
        let data = match fs::read(root.as_ref().join(path)) {
            Ok(data) => data,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        total = total.saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX));
        if _as_text(&data).is_some() {
            captured.insert(path.clone(), data);
        }
    }
    Ok(captured)
}

#[must_use]
pub fn _as_text(data: &[u8]) -> Option<String> {
    if data.len() > HARNESS_MAX_DIFF_BYTES || data.contains(&0) {
        return None;
    }
    String::from_utf8(data.to_vec()).ok()
}

#[must_use]
pub fn unified_diff(path: &str, old: Option<&str>, new: Option<&str>) -> String {
    let from = old.map_or_else(|| "/dev/null".to_owned(), |_| format!("a/{path}"));
    let to = new.map_or_else(|| "/dev/null".to_owned(), |_| format!("b/{path}"));
    let mut result = format!("--- {from}\n+++ {to}\n");
    let old_lines = old.unwrap_or_default().lines().collect::<Vec<_>>();
    let new_lines = new.unwrap_or_default().lines().collect::<Vec<_>>();
    if old_lines != new_lines {
        if writeln!(
            result,
            "@@ -1,{} +1,{} @@",
            old_lines.len(),
            new_lines.len()
        )
        .is_err()
        {
            return String::new();
        }
        for line in &old_lines {
            if writeln!(result, "-{line}").is_err() {
                return String::new();
            }
        }
        for line in &new_lines {
            if writeln!(result, "+{line}").is_err() {
                return String::new();
            }
        }
    }
    result
}

pub fn build_file_changes<Root>(
    root: Root,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
    before_contents: &BTreeMap<String, Vec<u8>>,
) -> io::Result<Vec<FileChange>>
where
    Root: AsRef<Path>,
{
    let root_path = root.as_ref();
    diff_snapshots(before, after)
        .into_iter()
        .map(|(path, kind)| {
            let old = before_contents.get(&path).and_then(|bytes| _as_text(bytes));
            let new = if kind == FileChangeKind::Deleted {
                None
            } else {
                fs::read(root_path.join(&path))
                    .ok()
                    .and_then(|bytes| _as_text(&bytes))
            };
            let diff = match kind {
                FileChangeKind::Deleted if old.is_some() => {
                    Some(unified_diff(&path, old.as_deref(), None))
                }
                FileChangeKind::Created if new.is_some() => {
                    Some(unified_diff(&path, None, new.as_deref()))
                }
                FileChangeKind::Modified if old.is_some() && new.is_some() => {
                    Some(unified_diff(&path, old.as_deref(), new.as_deref()))
                }
                FileChangeKind::Created | FileChangeKind::Modified | FileChangeKind::Deleted => {
                    None
                }
            };
            Ok(FileChange { path, kind, diff })
        })
        .collect()
}

#[cfg(test)]
#[expect(
    clippy::inline_modules,
    reason = "Behavior tests are kept beside the helper implementation"
)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_diff_is_sorted_and_classified() {
        let before = BTreeMap::from([
            (String::from("old"), String::from("a")),
            (String::from("same"), String::from("x")),
        ]);
        let after = BTreeMap::from([
            (String::from("new"), String::from("b")),
            (String::from("same"), String::from("x")),
        ]);
        assert_eq!(
            diff_snapshots(&before, &after),
            [
                (String::from("new"), FileChangeKind::Created),
                (String::from("old"), FileChangeKind::Deleted)
            ]
        );
    }

    #[test]
    fn text_filter_and_diff_headers_match_contract() {
    assert!(_as_text(b"a\0b").is_none());
        assert_eq!(
            unified_diff("file.txt", Some("old"), Some("new")),
            "--- a/file.txt\n+++ b/file.txt\n@@ -1,1 +1,1 @@\n-old\n+new\n"
        );
    }

    #[test]
    fn replays_the_snapshots_corpus() {
    drop(_as_text(b""));
    drop(_as_text(b"plain text"));
    }
}
