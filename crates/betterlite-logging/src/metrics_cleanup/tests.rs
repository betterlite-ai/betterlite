#![cfg(test)]

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::{self, Command};
use std::sync::{Arc, Mutex, PoisonError};

use rstest::rstest;
use tempfile::TempDir;
use tracing::subscriber::with_default;

use super::{
    CleanupError, is_running, live_gauge_pid, mark_dead_workers, mark_worker_exit, wipe_directory,
};

struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut captured = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        captured.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn logs_of<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let shared = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&shared);
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_writer(move || Captured(Arc::clone(&sink)))
        .finish();
    let result = with_default(subscriber, action);
    let text = String::from_utf8(shared.lock().unwrap().clone()).unwrap();
    (result, text)
}

fn touch(folder: &Path, name: &str) {
    fs::write(folder.join(name), b"samples").unwrap();
}

fn touch_all<S>(folder: &Path, names: &[S])
where
    S: AsRef<str>,
{
    for name in names {
        touch(folder, name.as_ref());
    }
}

// a folder where a file is expected makes remove_file fail even for root, which CI runs as
fn blocked(folder: &Path, name: &str) {
    fs::create_dir_all(folder.join(name)).unwrap();
    touch(&folder.join(name), "inside");
}

fn dead_pid() -> u32 {
    let mut child = Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

#[test]
fn wipe_directory_removes_every_db_file_and_logs_the_count() {
    let folder = TempDir::new().unwrap();
    touch_all(
        folder.path(),
        &[
            "counter_1.db",
            "gauge_liveall_2.db",
            ".hidden.db",
            "notes.txt",
            "db",
        ],
    );
    let (deleted, logs) = logs_of(|| wipe_directory(folder.path()).unwrap());
    assert_eq!(deleted, 2);
    assert!(!folder.path().join("counter_1.db").exists());
    assert!(!folder.path().join("gauge_liveall_2.db").exists());
    assert!(folder.path().join(".hidden.db").exists());
    assert!(folder.path().join("notes.txt").exists());
    assert!(folder.path().join("db").exists());
    let expected = format!(
        "Prometheus cleanup: wiped 2 stale .db files from {}",
        folder.path().display()
    );
    assert!(logs.contains(&expected), "{logs}");
}

#[test]
fn wipe_directory_warns_and_keeps_going_when_a_file_cannot_be_removed() {
    let folder = TempDir::new().unwrap();
    blocked(folder.path(), "stuck.db");
    touch(folder.path(), "counter_1.db");
    let (deleted, logs) = logs_of(|| wipe_directory(folder.path()).unwrap());
    assert_eq!(deleted, 1);
    assert!(!folder.path().join("counter_1.db").exists());
    assert!(folder.path().join("stuck.db").exists());
    let warning = format!(
        "WARN betterlite_logging::metrics_cleanup: Failed to delete stale prometheus file {}: ",
        folder.path().join("stuck.db").display()
    );
    assert!(logs.contains(&warning), "{logs}");
    assert!(logs.contains("wiped 1 stale .db files"), "{logs}");
}

#[test]
fn wipe_directory_logs_nothing_when_nothing_was_removed() {
    let folder = TempDir::new().unwrap();
    touch(folder.path(), "notes.txt");
    let (deleted, logs) = logs_of(|| wipe_directory(folder.path()).unwrap());
    assert_eq!(deleted, 0);
    assert_eq!(logs, "");
}

#[test]
fn wipe_directory_fails_when_the_folder_cannot_be_listed() {
    let folder = TempDir::new().unwrap();
    let missing = folder.path().join("missing");
    let error = wipe_directory(&missing).unwrap_err();
    assert!(matches!(&error, CleanupError::List { path, .. } if *path == missing));
}

#[rstest]
#[case::unset(None)]
#[case::empty(Some(""))]
fn mark_worker_exit_is_a_no_op_without_a_multiproc_dir(#[case] multiproc_dir: Option<&str>) {
    let ((), logs) = logs_of(|| mark_worker_exit(42, multiproc_dir.map(Path::new)));
    assert_eq!(logs, "");
}

#[test]
fn mark_worker_exit_drops_the_live_gauges_of_the_worker_and_logs_success() {
    let folder = TempDir::new().unwrap();
    touch_all(
        folder.path(),
        &[
            "gauge_liveall_42.db",
            "gauge_livesum_42.db",
            "gauge_liveall_43.db",
            "counter_42.db",
            "gauge_all_42.db",
        ],
    );
    let ((), logs) = logs_of(|| mark_worker_exit(42, Some(folder.path())));
    assert!(!folder.path().join("gauge_liveall_42.db").exists());
    assert!(!folder.path().join("gauge_livesum_42.db").exists());
    assert!(folder.path().join("gauge_liveall_43.db").exists());
    assert!(folder.path().join("counter_42.db").exists());
    assert!(folder.path().join("gauge_all_42.db").exists());
    assert!(
        logs.contains(
            "INFO betterlite_logging::metrics_cleanup: Prometheus cleanup: marked worker 42 as dead"
        ),
        "{logs}"
    );
}

#[test]
fn mark_worker_exit_warns_and_continues_when_mark_process_dead_fails() {
    let folder = TempDir::new().unwrap();
    blocked(folder.path(), "gauge_liveall_42.db");
    let ((), logs) = logs_of(|| mark_worker_exit(42, Some(folder.path())));
    assert!(
        logs.contains("WARN betterlite_logging::metrics_cleanup: Failed to mark prometheus worker 42 as dead: cannot remove"),
        "{logs}"
    );
    assert!(!logs.contains("marked worker 42 as dead"), "{logs}");
}

#[test]
fn is_running_is_true_for_this_process() {
    assert!(is_running(process::id()));
}

#[test]
fn is_running_is_true_for_a_process_another_user_may_own() {
    assert!(is_running(1));
}

#[test]
fn is_running_is_false_for_a_reaped_process() {
    assert!(!is_running(dead_pid()));
}

#[test]
fn is_running_is_false_past_the_pid_range() {
    let past_pid_t = u32::try_from(i32::MAX).unwrap().checked_add(1).unwrap();
    assert!(!is_running(past_pid_t));
}

// two dead workers beside the live writer, plus files that belong to neither of them
fn mixed_worker_folder() -> (TempDir, u32, [u32; 2]) {
    let folder = TempDir::new().unwrap();
    let mut dead = [dead_pid(), dead_pid()];
    dead.sort_unstable();
    let own = dead_pid();
    let [first, second] = dead;
    touch_all(
        folder.path(),
        &[
            format!("gauge_livesum_{second}.db"),
            format!("gauge_liveall_{first}.db"),
            format!("gauge_livemax_{first}.db"),
            format!("gauge_liveall_{own}.db"),
            "gauge_liveall_1.db".to_owned(),
            format!("counter_{first}.db"),
            format!("gauge_all_{first}.db"),
        ],
    );
    (folder, own, dead)
}

#[test]
fn mark_dead_workers_marks_only_dead_owners_and_returns_them_sorted() {
    let (folder, own, dead) = mixed_worker_folder();
    let [first, second] = dead;
    let (marked, logs) = logs_of(|| mark_dead_workers(folder.path(), own).unwrap());
    assert_eq!(marked, vec![first, second]);
    let expected = format!(
        "Prometheus cleanup: marked dead workers [{first}, {second}] in {}",
        folder.path().display()
    );
    assert!(logs.contains(&expected), "{logs}");
}

#[test]
fn mark_dead_workers_leaves_the_live_and_foreign_files_alone() {
    let (folder, own, dead) = mixed_worker_folder();
    let [first, second] = dead;
    mark_dead_workers(folder.path(), own).unwrap();
    for gone in [
        format!("gauge_livesum_{second}.db"),
        format!("gauge_liveall_{first}.db"),
        format!("gauge_livemax_{first}.db"),
    ] {
        assert!(!folder.path().join(gone.as_str()).exists(), "{gone}");
    }
    for kept in [
        format!("gauge_liveall_{own}.db"),
        "gauge_liveall_1.db".to_owned(),
        format!("counter_{first}.db"),
        format!("gauge_all_{first}.db"),
    ] {
        assert!(folder.path().join(kept.as_str()).exists(), "{kept}");
    }
}

#[test]
fn mark_dead_workers_returns_nothing_and_logs_nothing_for_an_empty_folder() {
    let folder = TempDir::new().unwrap();
    let (marked, logs) = logs_of(|| mark_dead_workers(folder.path(), process::id()).unwrap());
    assert_eq!(marked, Vec::<u32>::new());
    assert_eq!(logs, "");
}

#[test]
fn mark_dead_workers_fails_when_a_dead_worker_cannot_be_marked() {
    let folder = TempDir::new().unwrap();
    let pid = dead_pid();
    blocked(folder.path(), &format!("gauge_liveall_{pid}.db"));
    let error = mark_dead_workers(folder.path(), process::id()).unwrap_err();
    assert!(matches!(error, CleanupError::MarkDead { pid: failed, .. } if failed == pid));
}

#[test]
fn mark_dead_workers_fails_when_the_folder_cannot_be_listed() {
    let folder = TempDir::new().unwrap();
    let error = mark_dead_workers(&folder.path().join("missing"), process::id()).unwrap_err();
    assert!(matches!(error, CleanupError::List { .. }));
}

#[rstest]
#[case::live_all("gauge_liveall_12.db", Some(12))]
#[case::empty_mode("gauge_live_7.db", Some(7))]
#[case::leading_zeros("gauge_livesum_007.db", Some(7))]
#[case::mode_with_underscore_prefix("gauge_live_gauge_live_5.db", Some(5))]
#[case::uppercase_mode("gauge_liveALL_1.db", None)]
#[case::signed_pid("gauge_liveall_+1.db", None)]
#[case::two_numbers("gauge_liveall_1_2.db", None)]
#[case::no_pid("gauge_liveall_.db", None)]
#[case::not_live("gauge_all_3.db", None)]
#[case::prefixed("xgauge_liveall_3.db", None)]
#[case::other_suffix("gauge_liveall_3.db.tmp", None)]
#[case::no_underscore("gauge_live5.db", None)]
#[case::past_u32("gauge_liveall_4294967296.db", None)]
fn live_gauge_pid_reads_the_pid_the_way_litellm_matches_it(
    #[case] name: &str,
    #[case] expected: Option<u32>,
) {
    assert_eq!(live_gauge_pid(name), expected);
}
