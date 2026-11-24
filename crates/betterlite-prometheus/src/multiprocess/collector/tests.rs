#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::multiprocess::MultiprocessError;
use crate::multiprocess::collector::{MultiProcessCollector, mark_process_dead};
use crate::multiprocess::files::{DbEntry, encode_db_file};
use crate::scrape_endpoint::MetricSample;

fn key(metric: &str, sample: &str) -> String {
    let labels: BTreeMap<&str, &str> = BTreeMap::new();
    serde_json::to_string(&(metric, sample, labels, metric)).unwrap()
}

fn entry(metric: &str, sample: &str, value: f64) -> DbEntry {
    DbEntry {
        key: key(metric, sample),
        value,
        timestamp: 0.0,
    }
}

fn write_file(dir: &Path, name: &str, entries: &[DbEntry]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, encode_db_file(entries)).unwrap();
    path
}

fn write_live_files(dir: &Path, pid: u32) {
    let modes = ["liveall", "livemin", "livemax", "livesum", "livemostrecent"];
    for mode in modes {
        let name = format!("gauge_{mode}_{pid}.db");
        let _path = write_file(dir, &name, &[entry("queue_length", "queue_length", 1.0)]);
    }
}

#[test]
fn refuses_an_empty_path() {
    let error = MultiProcessCollector::new(Path::new("")).unwrap_err();
    assert!(matches!(error, MultiprocessError::MissingDirectory));
    assert_eq!(
        error.to_string(),
        "env PROMETHEUS_MULTIPROC_DIR is not set or not a directory"
    );
}

#[test]
fn refuses_a_file() {
    let dir = TempDir::new().unwrap();
    let path = write_file(dir.path(), "not_a_folder", &[]);
    let error = MultiProcessCollector::new(&path).unwrap_err();
    assert!(matches!(error, MultiprocessError::MissingDirectory));
}

#[test]
fn reads_every_db_file() {
    let dir = TempDir::new().unwrap();
    let _first = write_file(
        dir.path(),
        "counter_1001.db",
        &[entry("requests", "requests_total", 3.0)],
    );
    let _second = write_file(
        dir.path(),
        "counter_1002.db",
        &[entry("requests", "requests_total", 4.0)],
    );
    let collector = MultiProcessCollector::new(dir.path()).unwrap();
    let families = collector.collect().unwrap();
    assert_eq!(families.len(), 1);
    let family = families.first().unwrap();
    assert_eq!(
        family.samples,
        vec![MetricSample {
            name: "requests_total".to_owned(),
            labels: Vec::new(),
            value: 7.0,
        }]
    );
}

#[test]
fn skips_dot_files_and_files_that_do_not_end_in_db() {
    let dir = TempDir::new().unwrap();
    let _visible = write_file(
        dir.path(),
        "counter_1001.db",
        &[entry("visible", "visible_total", 1.0)],
    );
    let _hidden = write_file(
        dir.path(),
        ".counter_1002.db",
        &[entry("hidden", "hidden_total", 2.0)],
    );
    let _text = write_file(
        dir.path(),
        "counter_1003.txt",
        &[entry("text", "text_total", 3.0)],
    );
    let _notes = write_file(dir.path(), "notes", &[]);
    let collector = MultiProcessCollector::new(dir.path()).unwrap();
    let families = collector.collect().unwrap();
    let names: Vec<&str> = families.iter().map(|family| family.name.as_str()).collect();
    assert_eq!(names, vec!["visible"]);
}

#[test]
fn fails_when_a_listed_file_is_broken() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("counter_1001.db");
    fs::write(&path, b"garbage").unwrap();
    let collector = MultiProcessCollector::new(dir.path()).unwrap();
    let error = collector.collect().unwrap_err();
    assert!(matches!(error, MultiprocessError::Truncated { .. }));
}

#[test]
fn fails_when_the_folder_cannot_be_listed() {
    let dir = TempDir::new().unwrap();
    let collector = MultiProcessCollector::new(dir.path()).unwrap();
    fs::remove_dir_all(dir.path()).unwrap();
    let error = collector.collect().unwrap_err();
    assert!(matches!(error, MultiprocessError::List { .. }));
}

#[test]
fn mark_process_dead_removes_only_the_live_files_of_that_pid() {
    let dir = TempDir::new().unwrap();
    write_live_files(dir.path(), 7);
    let _all = write_file(
        dir.path(),
        "gauge_all_7.db",
        &[entry("queue_length", "queue_length", 1.0)],
    );
    let _other_pid = write_file(
        dir.path(),
        "gauge_livesum_8.db",
        &[entry("queue_length", "queue_length", 1.0)],
    );
    let _counter = write_file(
        dir.path(),
        "counter_7.db",
        &[entry("requests", "requests_total", 1.0)],
    );
    mark_process_dead(7, dir.path()).unwrap();
    assert!(!dir.path().join("gauge_liveall_7.db").exists());
    assert!(!dir.path().join("gauge_livemin_7.db").exists());
    assert!(!dir.path().join("gauge_livemax_7.db").exists());
    assert!(!dir.path().join("gauge_livesum_7.db").exists());
    assert!(!dir.path().join("gauge_livemostrecent_7.db").exists());
    assert!(dir.path().join("gauge_all_7.db").exists());
    assert!(dir.path().join("gauge_livesum_8.db").exists());
    assert!(dir.path().join("counter_7.db").exists());
}

#[test]
fn mark_process_dead_tolerates_a_pid_with_no_files() {
    let dir = TempDir::new().unwrap();
    mark_process_dead(7, dir.path()).unwrap();
}

#[test]
fn mark_process_dead_fails_when_a_live_file_cannot_be_removed() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("gauge_livesum_7.db");
    fs::create_dir_all(&path).unwrap();
    let error = mark_process_dead(7, dir.path()).unwrap_err();
    assert!(matches!(error, MultiprocessError::Remove { .. }));
}
