#![cfg(test)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;
use std::{env, fs, future, io, os, process};

use rstest::rstest;
use tokio::io::{Empty, Sink};

use crate::errors::{CappedOperation, HarnessError};
use crate::events::FileChange;
use crate::sandbox::{CompletedRun, Process, Sandbox, SandboxCommand};
use crate::snapshots::{
    HARNESS_MAX_DIFF_BYTES, HARNESS_SNAPSHOT_MAX_FILE_BYTES, HARNESS_SNAPSHOT_MAX_TOTAL_BYTES,
    build_file_changes, capture_text_contents, diff_snapshots, snapshot_local, snapshot_local_sync,
    unified_diff,
};
use crate::types::FileChangeKind;

const HELLO_SHA256: &str = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";

struct NoProcess {
    stdout: Empty,
    stderr: Empty,
}

impl Process for NoProcess {
    type Stdin = Sink;
    type Stdout = Empty;
    type Stderr = Empty;

    fn stdin(&mut self) -> Option<&mut Sink> {
        None
    }

    fn stdout(&mut self) -> &mut Empty {
        &mut self.stdout
    }

    fn stderr(&mut self) -> &mut Empty {
        &mut self.stderr
    }

    fn wait(&mut self) -> impl Future<Output = Result<i32, HarnessError>> + Send {
        future::ready(Ok(0_i32))
    }

    fn kill(&mut self) -> impl Future<Output = Result<(), HarnessError>> + Send {
        future::ready(Ok(()))
    }
}

struct MemorySandbox {
    workdir: String,
    files: BTreeMap<String, Vec<u8>>,
}

impl Sandbox for MemorySandbox {
    type Process = NoProcess;

    fn workdir(&self) -> &str {
        &self.workdir
    }

    fn exec(
        &self,
        _command: &SandboxCommand,
    ) -> impl Future<Output = Result<NoProcess, HarnessError>> + Send {
        future::ready(Err(HarnessError::SessionClosed))
    }

    fn run(
        &self,
        _command: &SandboxCommand,
        _timeout: Duration,
        _max_output_bytes: usize,
    ) -> impl Future<Output = Result<CompletedRun, HarnessError>> + Send {
        future::ready(Err(HarnessError::SessionClosed))
    }

    fn read(
        &self,
        path: &str,
        max_bytes: usize,
    ) -> impl Future<Output = Result<Vec<u8>, HarnessError>> + Send {
        let found = self.files.get(path).ok_or_else(|| HarnessError::Sandbox {
            source: io::Error::from(io::ErrorKind::NotFound),
        });
        future::ready(found.and_then(|data| {
            if data.len() > max_bytes {
                Err(HarnessError::ContentLimitExceeded {
                    operation: CappedOperation::Read,
                    limit: max_bytes,
                })
            } else {
                Ok(data.clone())
            }
        }))
    }

    fn write(
        &self,
        _path: &str,
        _data: &[u8],
    ) -> impl Future<Output = Result<(), HarnessError>> + Send {
        future::ready(Err(HarnessError::SessionClosed))
    }

    fn host_url(&self, port: u16) -> String {
        format!("http://127.0.0.1:{port}")
    }

    fn which(
        &self,
        _binary: &str,
    ) -> impl Future<Output = Result<Option<String>, HarnessError>> + Send {
        future::ready(Ok(None))
    }

    fn snapshot(
        &self,
    ) -> impl Future<Output = Result<BTreeMap<String, String>, HarnessError>> + Send {
        future::ready(Ok(BTreeMap::new()))
    }

    fn tempdir(&self) -> impl Future<Output = Result<String, HarnessError>> + Send {
        future::ready(Err(HarnessError::SessionClosed))
    }

    fn close(&self) -> impl Future<Output = Result<(), HarnessError>> + Send {
        future::ready(Ok(()))
    }
}

fn fresh_workspace(name: &str) -> PathBuf {
    let root = env::temp_dir().join(format!("betterlite-snapshots-{}-{name}", process::id()));
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&root).unwrap();
    root
}

fn hashes(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(path, digest)| ((*path).to_owned(), (*digest).to_owned()))
        .collect()
}

fn sandbox_with(files: &[(&str, &[u8])]) -> MemorySandbox {
    MemorySandbox {
        workdir: "/work".to_owned(),
        files: files
            .iter()
            .map(|(path, data)| ((*path).to_owned(), data.to_vec()))
            .collect(),
    }
}

#[test]
fn snapshot_hashes_regular_files_by_relative_path() {
    let root = fresh_workspace("hash");
    fs::create_dir_all(root.join("src/nested")).unwrap();
    fs::write(root.join("hello.txt"), "hello\n").unwrap();
    fs::write(root.join("src/nested/hello.txt"), "hello\n").unwrap();
    os::unix::fs::symlink(root.join("hello.txt"), root.join("linked.txt")).unwrap();
    os::unix::fs::symlink(root.join("src"), root.join("linked-src")).unwrap();
    let expected = hashes(&[
        ("hello.txt", HELLO_SHA256),
        ("src/nested/hello.txt", HELLO_SHA256),
    ]);
    assert_eq!(snapshot_local_sync(&root), expected);
}

#[rstest]
#[case::git(".git")]
#[case::node_modules("node_modules")]
#[case::dot_venv(".venv")]
#[case::venv("venv")]
#[case::pycache("__pycache__")]
#[case::mypy_cache(".mypy_cache")]
#[case::pytest_cache(".pytest_cache")]
#[case::ruff_cache(".ruff_cache")]
fn snapshot_skips_tool_folders_at_any_depth(#[case] skipped: &str) {
    let root = fresh_workspace(&format!("skip{skipped}"));
    fs::create_dir_all(root.join(skipped)).unwrap();
    fs::create_dir_all(root.join("pkg").join(skipped)).unwrap();
    fs::write(root.join(skipped).join("hello.txt"), "hello\n").unwrap();
    fs::write(root.join("pkg").join(skipped).join("hello.txt"), "hello\n").unwrap();
    fs::write(root.join("pkg/hello.txt"), "hello\n").unwrap();
    fs::write(root.join(format!("{skipped}.txt")), "hello\n").unwrap();
    let expected = hashes(&[
        ("pkg/hello.txt", HELLO_SHA256),
        (&format!("{skipped}.txt"), HELLO_SHA256),
    ]);
    assert_eq!(snapshot_local_sync(&root), expected);
}

#[rstest]
#[case::at_the_limit(0, true)]
#[case::past_the_limit(1, false)]
fn snapshot_leaves_out_huge_files(#[case] extra: usize, #[case] kept: bool) {
    let root = fresh_workspace(&format!("huge{extra}"));
    let size = u64::try_from(HARNESS_SNAPSHOT_MAX_FILE_BYTES.checked_add(extra).unwrap()).unwrap();
    fs::File::create(root.join("big.bin"))
        .unwrap()
        .set_len(size)
        .unwrap();
    assert_eq!(snapshot_local_sync(&root).contains_key("big.bin"), kept);
}

#[tokio::test]
async fn snapshot_runs_off_the_async_thread() {
    let root = fresh_workspace("async");
    fs::write(root.join("hello.txt"), "hello\n").unwrap();
    let snapshot = snapshot_local(root).await.unwrap();
    assert_eq!(snapshot, hashes(&[("hello.txt", HELLO_SHA256)]));
}

#[test]
fn a_missing_root_has_an_empty_snapshot() {
    let root = fresh_workspace("missing").join("absent");
    assert!(snapshot_local_sync(&root).is_empty());
}

#[test]
fn diff_lists_changes_sorted_by_path() {
    let before = hashes(&[("b.txt", "1"), ("gone.txt", "2"), ("same.txt", "3")]);
    let after = hashes(&[("a.txt", "4"), ("b.txt", "5"), ("same.txt", "3")]);
    assert_eq!(
        diff_snapshots(&before, &after),
        [
            ("a.txt".to_owned(), FileChangeKind::Created),
            ("b.txt".to_owned(), FileChangeKind::Modified),
            ("gone.txt".to_owned(), FileChangeKind::Deleted),
        ]
    );
}

#[rstest]
#[case::replaced_without_newline(
    Some("old"),
    Some("new"),
    "--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n"
)]
#[case::created(
    None,
    Some("a\nb\n"),
    "--- /dev/null\n+++ b/f.txt\n@@ -0,0 +1,2 @@\n+a\n+b\n"
)]
#[case::deleted(Some("a\n"), None, "--- a/f.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n")]
#[case::three_lines_of_context(
    Some("1\n2\n3\n4\n5\n6\n7\n8\n9\n"),
    Some("1\n2\n3\n4\nfive\n6\n7\n8\n9\n"),
    "--- a/f.txt\n+++ b/f.txt\n@@ -2,7 +2,7 @@\n 2\n 3\n 4\n-5\n+five\n 6\n 7\n 8\n"
)]
#[case::newline_added(
    Some("a"),
    Some("a\n"),
    "--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+a\n"
)]
#[case::identical(Some("a\n"), Some("a\n"), "")]
#[case::created_empty(None, Some(""), "")]
fn builds_unified_diffs(
    #[case] old: Option<&str>,
    #[case] new: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(unified_diff("f.txt", old, new), expected);
}

#[rstest]
#[case::text("a.txt", b"hello\n", true)]
#[case::binary("a.txt", b"a\0b", false)]
#[case::not_utf8("a.txt", b"caf\xe9", false)]
#[case::missing("elsewhere.txt", b"hello\n", false)]
fn captures_small_text_files(#[case] stored: &str, #[case] data: &[u8], #[case] kept: bool) {
    let sandbox = sandbox_with(&[(stored, data)]);
    let contents = futures_util::FutureExt::now_or_never(capture_text_contents(
        &sandbox,
        &hashes(&[("a.txt", "1")]),
    ))
    .unwrap();
    let expected = if kept {
        BTreeMap::from([("a.txt".to_owned(), data.to_vec())])
    } else {
        BTreeMap::new()
    };
    assert_eq!(contents, expected);
}

#[rstest]
#[case::at_the_diff_limit(HARNESS_MAX_DIFF_BYTES, true)]
#[case::past_the_diff_limit(HARNESS_MAX_DIFF_BYTES + 1, false)]
fn captures_files_up_to_the_diff_limit(#[case] size: usize, #[case] kept: bool) {
    let text = vec![b'x'; size];
    let sandbox = sandbox_with(&[("a.txt", &text)]);
    let contents = futures_util::FutureExt::now_or_never(capture_text_contents(
        &sandbox,
        &hashes(&[("a.txt", "1")]),
    ))
    .unwrap();
    assert_eq!(contents.contains_key("a.txt"), kept);
}

#[rstest]
#[case::budget_spent(HARNESS_SNAPSHOT_MAX_TOTAL_BYTES, false)]
#[case::budget_left(HARNESS_SNAPSHOT_MAX_TOTAL_BYTES - 1, true)]
fn capture_stops_at_the_total_budget(#[case] first_size: usize, #[case] later_kept: bool) {
    let first = vec![b'x'; first_size];
    let sandbox = sandbox_with(&[("a.txt", &first), ("b.txt", b"later\n")]);
    let contents = futures_util::FutureExt::now_or_never(capture_text_contents(
        &sandbox,
        &hashes(&[("a.txt", "1"), ("b.txt", "2")]),
    ))
    .unwrap();
    assert_eq!(contents.contains_key("b.txt"), later_kept);
    assert!(!contents.contains_key("a.txt"));
}

#[rstest]
#[case::created_text(
    FileChangeKind::Created,
    None,
    Some(b"new\n".as_slice()),
    Some("--- /dev/null\n+++ b/f.txt\n@@ -0,0 +1 @@\n+new\n")
)]
#[case::created_binary(FileChangeKind::Created, None, Some(b"\0\x01".as_slice()), None)]
#[case::created_unreadable(FileChangeKind::Created, None, None, None)]
#[case::modified_text(
    FileChangeKind::Modified,
    Some(b"old\n".as_slice()),
    Some(b"new\n".as_slice()),
    Some("--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-old\n+new\n")
)]
#[case::modified_without_old(FileChangeKind::Modified, None, Some(b"new\n".as_slice()), None)]
#[case::modified_into_binary(FileChangeKind::Modified, Some(b"old\n".as_slice()), Some(b"\0".as_slice()), None)]
#[case::deleted_text(
    FileChangeKind::Deleted,
    Some(b"gone\n".as_slice()),
    Some(b"still here\n".as_slice()),
    Some("--- a/f.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\n")
)]
#[case::deleted_binary(FileChangeKind::Deleted, Some(b"\0".as_slice()), None, None)]
#[case::deleted_unknown(FileChangeKind::Deleted, None, None, None)]
fn file_changes_diff_text_on_both_sides(
    #[case] kind: FileChangeKind,
    #[case] old: Option<&[u8]>,
    #[case] current: Option<&[u8]>,
    #[case] diff: Option<&str>,
) {
    let before = if kind == FileChangeKind::Created {
        hashes(&[])
    } else {
        hashes(&[("f.txt", "1")])
    };
    let after = if kind == FileChangeKind::Deleted {
        hashes(&[])
    } else {
        hashes(&[("f.txt", "2")])
    };
    let before_contents = old.map_or_else(BTreeMap::new, |data| {
        BTreeMap::from([("f.txt".to_owned(), data.to_vec())])
    });
    let sandbox = current.map_or_else(
        || sandbox_with(&[]),
        |data| sandbox_with(&[("f.txt", data)]),
    );
    let changes = futures_util::FutureExt::now_or_never(build_file_changes(
        &sandbox,
        &before,
        &after,
        &before_contents,
    ))
    .unwrap();
    let expected = FileChange {
        path: "f.txt".to_owned(),
        kind,
        diff: diff.map(str::to_owned),
    };
    assert_eq!(changes, [expected]);
}

#[test]
fn a_created_file_too_large_to_diff_has_no_diff() {
    let large = vec![b'x'; HARNESS_MAX_DIFF_BYTES.checked_add(1).unwrap()];
    let sandbox = sandbox_with(&[("large.txt", &large)]);
    let after = hashes(&[("large.txt", "1")]);
    let changes = futures_util::FutureExt::now_or_never(build_file_changes(
        &sandbox,
        &BTreeMap::new(),
        &after,
        &BTreeMap::new(),
    ))
    .unwrap();
    let expected = FileChange {
        path: "large.txt".to_owned(),
        kind: FileChangeKind::Created,
        diff: None,
    };
    assert_eq!(changes, [expected]);
}

#[test]
fn file_changes_follow_the_sorted_diff() {
    let before = hashes(&[("b.txt", "1"), ("same.txt", "2"), ("c.txt", "3")]);
    let after = hashes(&[("a.txt", "4"), ("same.txt", "2"), ("c.txt", "5")]);
    let sandbox = sandbox_with(&[]);
    let changes = futures_util::FutureExt::now_or_never(build_file_changes(
        &sandbox,
        &before,
        &after,
        &BTreeMap::new(),
    ))
    .unwrap();
    let listed = changes
        .iter()
        .map(|change| (change.path.as_str(), change.kind))
        .collect::<Vec<_>>();
    assert_eq!(
        listed,
        [
            ("a.txt", FileChangeKind::Created),
            ("b.txt", FileChangeKind::Deleted),
            ("c.txt", FileChangeKind::Modified),
        ]
    );
}

#[rstest]
#[case::changed(
    "changed",
    "--- a/fuzz.txt\n+++ b/fuzz.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+three\n\\ No newline at end of file\n"
)]
#[case::deleted(
    "deleted",
    "--- a/fuzz.txt\n+++ b/fuzz.txt\n@@ -1 +0,0 @@\n-only old\n"
)]
#[case::same("same", "")]
#[case::text("text", "--- a/fuzz.txt\n+++ b/fuzz.txt\n@@ -1 +0,0 @@\n-plain text\n")]
fn replays_the_snapshots_corpus(#[case] seed: &str, #[case] expected: &str) {
    let folder = concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/snapshots");
    assert_eq!(fs::read_dir(folder).unwrap().count(), 4);
    let text = fs::read_to_string(format!("{folder}/{seed}")).unwrap();
    let (old, new) = text.split_once('\0').unwrap_or((&text, ""));
    assert_eq!(unified_diff("fuzz.txt", Some(old), Some(new)), expected);
}
