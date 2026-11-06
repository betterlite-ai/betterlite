#![cfg(test)]

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use rstest::rstest;
use serde_json::Value;
use tokio::time::{sleep, timeout};

use super::files::{
    FileAccess, LocalFiles, MAX_CONCURRENT_FILE_OPERATIONS, PrivateFiles, StagedTemp,
};
use super::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps, safe_json_loads};

mod private_json_tests_behavior;

fn value(text: &str) -> Value {
    serde_json::from_str(text).expect("valid json")
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path)
        .expect("reads metadata")
        .permissions()
        .mode()
        & 0o777
}

fn entries_of(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .expect("reads the directory")
        .map(|entry| {
            entry
                .expect("directory entry")
                .file_name()
                .into_string()
                .expect("a utf-8 file name")
        })
        .collect();
    names.sort_unstable();
    names
}

// A blocking gate for the fake file layer, so a test can hold blocking jobs open and cancel their callers
#[derive(Default)]
struct Gate {
    open: AtomicBool,
    entered: AtomicUsize,
}

impl Gate {
    fn wait(&self) {
        self.entered.fetch_add(1, Ordering::SeqCst);
        while !self.open.load(Ordering::SeqCst) {
            thread::park_timeout(Duration::from_millis(1));
        }
    }

    fn release(&self) {
        self.open.store(true, Ordering::SeqCst);
    }

    fn entered(&self) -> usize {
        self.entered.load(Ordering::SeqCst)
    }
}

async fn wait_until_entered(gate: &Gate, count: usize) {
    for _ in 0_usize..200_usize {
        if gate.entered() >= count {
            return;
        }
        sleep(Duration::from_millis(10)).await;
    }
}

// Each call is dropped after a short timeout, so its joined future is cancelled while its blocking job stays open
async fn cancel_gated_calls(files: &PrivateFiles, directory: &Path) {
    for _ in 0_usize..MAX_CONCURRENT_FILE_OPERATIONS {
        let call = files.ensure_private_dir(directory);
        let _cancelled = timeout(Duration::from_millis(50), call).await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    None,
    Write,
    WriteAndRemove,
    Replace,
}

struct ScriptedFiles {
    fault: Fault,
    gate: Option<Arc<Gate>>,
}

impl ScriptedFiles {
    fn gated(gate: Arc<Gate>) -> Self {
        Self {
            fault: Fault::None,
            gate: Some(gate),
        }
    }
}

impl FileAccess for ScriptedFiles {
    fn create_parents(&self, directory: &Path) -> io::Result<()> {
        LocalFiles.create_parents(directory)
    }

    fn ensure_private_directory(&self, directory: &Path) -> io::Result<()> {
        if let Some(gate) = &self.gate {
            gate.wait();
        }
        LocalFiles.ensure_private_directory(directory)
    }

    fn directory_mode(&self, directory: &Path) -> io::Result<u32> {
        LocalFiles.directory_mode(directory)
    }

    fn set_mode(&self, path: &Path, mode: u32) -> io::Result<()> {
        LocalFiles.set_mode(path, mode)
    }

    fn create_staged(&self, directory: &Path, suffix: Option<&str>) -> io::Result<StagedTemp> {
        LocalFiles.create_staged(directory, suffix)
    }

    fn write_staged(&self, staged: &mut StagedTemp, bytes: &[u8]) -> io::Result<()> {
        if matches!(self.fault, Fault::Write | Fault::WriteAndRemove) {
            return Err(io::Error::other("scripted write failure"));
        }
        LocalFiles.write_staged(staged, bytes)
    }

    fn replace(&self, staged: &Path, target: &Path) -> io::Result<()> {
        if self.fault == Fault::Replace {
            return Err(io::Error::other("scripted replace failure"));
        }
        LocalFiles.replace(staged, target)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        if self.fault == Fault::WriteAndRemove {
            return Err(io::Error::other("scripted remove failure"));
        }
        LocalFiles.remove(path)
    }

    fn rewrite_existing(&self, target: &Path, bytes: &[u8]) -> io::Result<()> {
        LocalFiles.rewrite_existing(target, bytes)
    }
}

#[rstest]
fn replays_the_private_json_corpus(#[files("fuzz/corpus/private_json/*")] path: PathBuf) {
    let bytes = fs::read(&path).expect("corpus file");
    if let Ok(parsed) = safe_json_loads(&bytes) {
        let dumped = safe_dumps(&parsed, DEFAULT_MAX_RECURSE_DEPTH, None).expect("round trip");
        let _restored: Value = serde_json::from_str(&dumped).expect("valid json");
    }
}
